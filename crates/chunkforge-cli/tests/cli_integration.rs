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
    assert!(help_s.contains("archive"), "{help_s}");
    assert!(help_s.contains("extract"), "{help_s}");
    assert!(help_s.contains("cat"), "{help_s}");
    assert!(help_s.contains("verify"), "{help_s}");
    assert!(help_s.contains("diff"), "{help_s}");

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

// --- Phase 2 M3: --source / --cache ---

use std::thread;
use std::time::Duration;
use tiny_http::{Header, Method, Response, Server, StatusCode};

fn spawn_static_store_server(store_root: PathBuf) -> (String, thread::JoinHandle<()>) {
    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let handle = thread::spawn(move || {
        for request in server.incoming_requests() {
            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(&url);
            let rel = path.trim_start_matches('/');
            let file_path = store_root.join(rel);

            if request.method() == &Method::Head || request.method() == &Method::Get {
                if file_path.is_file() {
                    let data = fs::read(&file_path).unwrap_or_default();
                    if request.method() == &Method::Head {
                        let response = Response::empty(200).with_header(
                            Header::from_bytes(&b"Content-Length"[..], data.len().to_string())
                                .unwrap(),
                        );
                        let _ = request.respond(response);
                    } else {
                        let _ = request.respond(Response::from_data(data));
                    }
                } else {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
            } else {
                let _ = request.respond(Response::empty(StatusCode(405)));
            }
        }
    });
    thread::sleep(Duration::from_millis(20));
    (base, handle)
}

#[test]
fn help_mentions_source_and_cache() {
    let cat = run_ok(&["cat", "--help"]);
    let cat_s = String::from_utf8_lossy(&cat.stdout);
    assert!(cat_s.contains("--source"), "{cat_s}");
    assert!(cat_s.contains("--cache"), "{cat_s}");
    assert!(cat_s.contains("--store"), "{cat_s}");

    let ver = run_ok(&["verify", "--help"]);
    let ver_s = String::from_utf8_lossy(&ver.stdout);
    assert!(ver_s.contains("--source"), "{ver_s}");
    assert!(ver_s.contains("--cache"), "{ver_s}");
}

#[test]
fn source_path_synonym_for_store() {
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
    run_ok(&[
        "verify",
        "--source",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    run_ok(&[
        "cat",
        "--source",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out).unwrap());
}

#[test]
fn http_source_verify_fills_empty_cache_then_cat_from_cache() {
    let dir = tempdir().unwrap();
    let remote_store = dir.path().join("remote");
    let cache = dir.path().join("cache");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("from-cache");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        remote_store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    assert_eq!(count_cnk(&cache.join("chunks")), 0);

    let (base, _handle) = spawn_static_store_server(remote_store.clone());

    // Remote has chunks; cache empty → verify succeeds and fills cache.
    run_ok(&[
        "verify",
        "--source",
        &base,
        "--cache",
        cache.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);

    let cached = count_cnk(&cache.join("chunks"));
    assert!(
        cached >= 1,
        "cache should gain .cnk files after verify, got {cached}"
    );

    // After fill, cat from cache-only (no HTTP) works.
    run_ok(&[
        "cat",
        "--source",
        cache.to_str().unwrap(),
        idx.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out).unwrap());
}

#[test]
fn http_source_cat_with_cache() {
    let dir = tempdir().unwrap();
    let remote_store = dir.path().join("remote");
    let cache = dir.path().join("cache");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("reassembled");
    let input = fixtures_dir().join("binary-256.bin");

    run_ok(&[
        "make",
        "--store",
        remote_store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let (base, _handle) = spawn_static_store_server(remote_store);

    run_ok(&[
        "cat",
        "--source",
        &base,
        "--cache",
        cache.to_str().unwrap(),
        idx.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out).unwrap());
    assert!(count_cnk(&cache.join("chunks")) >= 1);
}

#[test]
fn file_url_source_works() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("out.bin");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let url = format!("file://{}", store.display());
    run_ok(&["verify", "--source", &url, idx.to_str().unwrap()]);
    run_ok(&[
        "cat",
        "--source",
        &url,
        idx.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out).unwrap());
}

// --- Phase 2 M5: mount CLI ---

#[test]
fn mount_help_lists_source_cache_name() {
    let help = run_ok(&["--help"]);
    let help_s = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_s.contains("mount"),
        "top-level help should list mount:\n{help_s}"
    );

    let m = run_ok(&["mount", "--help"]);
    let s = String::from_utf8_lossy(&m.stdout);
    assert!(s.contains("--source"), "{s}");
    assert!(s.contains("--cache"), "{s}");
    assert!(s.contains("--store"), "{s}");
    assert!(s.contains("--name"), "{s}");
    assert!(
        s.contains("--no-prefetch"),
        "mount --help must list --no-prefetch:\n{s}"
    );
    assert!(
        s.contains("--prefetch-chunks"),
        "mount --help must list --prefetch-chunks:\n{s}"
    );
    assert!(s.to_ascii_lowercase().contains("mountpoint"), "{s}");
}

#[test]
fn mount_rejects_both_store_and_source() {
    let dir = tempdir().unwrap();
    let mnt = dir.path().join("mnt");
    fs::create_dir(&mnt).unwrap();
    let idx = fixtures_dir().join("hello.txt"); // wrong type; clap should fail first on args
    let out = Command::new(bin())
        .args([
            "mount",
            "--store",
            dir.path().to_str().unwrap(),
            "--source",
            dir.path().to_str().unwrap(),
            idx.to_str().unwrap(),
            mnt.to_str().unwrap(),
        ])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
}

#[test]
fn mount_missing_mountpoint_dir_fails_clearly() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let missing = dir.path().join("no-such-mnt");
    let out = run_fail(&[
        "mount",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
        missing.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("mountpoint") || err.contains("does not exist") || err.contains("fuse"),
        "stderr={err}"
    );
}

/// Real FUSE mount via CLI — needs fuse3 + /dev/fuse; skipped by default in CI.
#[test]
#[ignore = "requires fuse3 + /dev/fuse; run with --ignored when available"]
fn mount_cli_hello_cmp_and_ro() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let mnt = dir.path().join("mnt");
    fs::create_dir(&mnt).unwrap();
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let mnt_s = mnt.to_str().unwrap().to_string();
    let store_s = store.to_str().unwrap().to_string();
    let idx_s = idx.to_str().unwrap().to_string();
    let bin_path = bin();

    let handle = thread::spawn(move || {
        Command::new(&bin_path)
            .args(["mount", "--store", &store_s, &idx_s, &mnt_s])
            .status()
    });

    let virtual_file = mnt.join("hello");
    for _ in 0..50 {
        if virtual_file.is_file() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert!(
        virtual_file.is_file(),
        "mount did not appear at {}",
        virtual_file.display()
    );
    assert_eq!(fs::read(&input).unwrap(), fs::read(&virtual_file).unwrap());
    assert!(
        fs::write(&virtual_file, b"x").is_err(),
        "write should fail on RO mount"
    );

    let _ = Command::new("fusermount3").args(["-u"]).arg(&mnt).status();
    let _ = Command::new("fusermount").args(["-u"]).arg(&mnt).status();
    let _ = handle.join();
}

// --- Phase 3 M3: --url-template / --prefix / --header ---

use std::sync::{Arc, Mutex};

#[test]
fn help_mentions_url_template_prefix_header() {
    for cmd in ["cat", "verify", "mount"] {
        let help = run_ok(&[cmd, "--help"]);
        let s = String::from_utf8_lossy(&help.stdout);
        assert!(
            s.contains("--url-template"),
            "{cmd} --help missing --url-template:\n{s}"
        );
        assert!(
            s.contains("--prefix"),
            "{cmd} --help missing --prefix:\n{s}"
        );
        assert!(
            s.contains("--header"),
            "{cmd} --help missing --header:\n{s}"
        );
    }
}

#[test]
fn verify_explicit_default_url_template_matches_bare_source() {
    let dir = tempdir().unwrap();
    let remote_store = dir.path().join("remote");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        remote_store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let (base, _handle) = spawn_static_store_server(remote_store);

    // Bare --source (0.2.0 behaviour).
    run_ok(&["verify", "--source", &base, idx.to_str().unwrap()]);

    // Explicit default template must behave identically.
    run_ok(&[
        "verify",
        "--source",
        &base,
        "--url-template",
        "{base}/{path}",
        idx.to_str().unwrap(),
    ]);
}

#[test]
fn verify_header_flag_observed_by_mock_server() {
    let dir = tempdir().unwrap();
    let remote_store = dir.path().join("remote");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        remote_store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let seen_auth = Arc::new(Mutex::new(None::<String>));
    let seen_auth2 = Arc::clone(&seen_auth);
    let store_root = remote_store.clone();

    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let _handle = thread::spawn(move || {
        for request in server.incoming_requests() {
            let auth = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Authorization"))
                .map(|h| h.value.as_str().to_string());
            if let Some(v) = auth {
                *seen_auth2.lock().unwrap() = Some(v);
            }

            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(&url);
            let rel = path.trim_start_matches('/');
            let file_path = store_root.join(rel);

            if request.method() == &Method::Head || request.method() == &Method::Get {
                if file_path.is_file() {
                    let data = fs::read(&file_path).unwrap_or_default();
                    if request.method() == &Method::Head {
                        let response = Response::empty(200).with_header(
                            Header::from_bytes(&b"Content-Length"[..], data.len().to_string())
                                .unwrap(),
                        );
                        let _ = request.respond(response);
                    } else {
                        let _ = request.respond(Response::from_data(data));
                    }
                } else {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
            } else {
                let _ = request.respond(Response::empty(StatusCode(405)));
            }
        }
    });
    thread::sleep(Duration::from_millis(20));

    run_ok(&[
        "verify",
        "--source",
        &base,
        "--header",
        "Authorization: Bearer cli-m3-token",
        idx.to_str().unwrap(),
    ]);

    let observed = seen_auth.lock().unwrap().clone();
    assert_eq!(
        observed.as_deref(),
        Some("Bearer cli-m3-token"),
        "mock server must observe Authorization from --header"
    );
}

#[test]
fn template_flags_rejected_for_local_source() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_fail(&[
        "verify",
        "--source",
        store.to_str().unwrap(),
        "--url-template",
        "{base}/{path}",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("http") && (err.contains("url-template") || err.contains("template")),
        "expected readable non-HTTP + template error, got: {err}"
    );

    let out2 = run_fail(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--header",
        "X-Test: 1",
        idx.to_str().unwrap(),
    ]);
    let err2 = String::from_utf8_lossy(&out2.stderr).to_lowercase();
    assert!(
        err2.contains("http") && (err2.contains("header") || err2.contains("template")),
        "expected readable non-HTTP + --header error, got: {err2}"
    );
}

#[test]
fn verify_prefix_url_template_against_prefixed_layout() {
    let dir = tempdir().unwrap();
    let cas = dir.path().join("cas");
    let mirror = dir.path().join("mirror");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        cas.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // Serve store contents under /data/… (S3-style prefix).
    let data_root = mirror.join("data");
    fs::create_dir_all(&data_root).unwrap();
    copy_dir_recursive(&cas, &data_root);

    let (base, _handle) = spawn_static_store_server(mirror);

    run_ok(&[
        "verify",
        "--source",
        &base,
        "--prefix",
        "data/",
        "--url-template",
        "{base}/{prefix}{path}",
        idx.to_str().unwrap(),
    ]);
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

// --- Phase 3 M4: doctor ---

/// Find the first `.cnk` under `chunks/` and return (absolute path, hex id).
fn first_cnk_id(chunks_root: &Path) -> (PathBuf, String) {
    let mut stack = vec![chunks_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("cnk") {
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap();
                let parent_hex = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap();
                assert_eq!(parent_hex.len(), 2, "shard dir must be 2 hex");
                assert_eq!(stem.len(), 62, "cnk stem must be 62 hex");
                let id = format!("{parent_hex}{stem}");
                return (path, id);
            }
        }
    }
    panic!("no .cnk under {}", chunks_root.display());
}

#[test]
fn doctor_help_lists_flags() {
    let help = run_ok(&["--help"]);
    let help_s = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_s.contains("doctor"),
        "top-level help should list doctor:\n{help_s}"
    );

    let d = run_ok(&["doctor", "--help"]);
    let s = String::from_utf8_lossy(&d.stdout);
    assert!(s.contains("--store"), "{s}");
    assert!(s.contains("--source"), "{s}");
    assert!(s.contains("--url-template"), "{s}");
    assert!(s.contains("--prefix"), "{s}");
    assert!(s.contains("--header"), "{s}");
    assert!(s.contains("--deep"), "{s}");
    assert!(s.contains("--no-probe"), "{s}");
    assert!(
        s.contains("--format"),
        "doctor --help should list --format:\n{s}"
    );
}

#[test]
fn doctor_complete_store_exits_zero() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("doctor: ok"), "stderr={err}");
    assert!(
        err.contains("meta.toml") || err.contains("store meta"),
        "stderr={err}"
    );
    assert!(
        out.stdout.is_empty(),
        "complete store should print no missing ids"
    );
}

#[test]
fn doctor_missing_chunk_nonzero_and_prints_id() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let (cnk_path, missing_id) = first_cnk_id(&store.join("chunks"));
    fs::remove_file(&cnk_path).unwrap();

    let out = run_fail(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.lines().any(|l| l.trim() == missing_id),
        "stdout must contain missing id {missing_id}, got:\n{stdout}"
    );
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(err.contains("missing"), "stderr={err}");
}

#[test]
fn doctor_http_source_uses_has_probing() {
    let dir = tempdir().unwrap();
    let remote_store = dir.path().join("remote");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        remote_store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let head_count = Arc::new(Mutex::new(0usize));
    let get_count = Arc::new(Mutex::new(0usize));
    let head_count2 = Arc::clone(&head_count);
    let get_count2 = Arc::clone(&get_count);
    let store_root = remote_store.clone();

    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let _handle = thread::spawn(move || {
        for request in server.incoming_requests() {
            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(&url);
            let rel = path.trim_start_matches('/');
            let file_path = store_root.join(rel);

            if request.method() == &Method::Head {
                *head_count2.lock().unwrap() += 1;
                if file_path.is_file() {
                    let data = fs::read(&file_path).unwrap_or_default();
                    let response = Response::empty(200).with_header(
                        Header::from_bytes(&b"Content-Length"[..], data.len().to_string()).unwrap(),
                    );
                    let _ = request.respond(response);
                } else if rel.is_empty() || rel == "/" {
                    // Base probe against "/" or empty path.
                    let _ = request.respond(Response::empty(404));
                } else {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
            } else if request.method() == &Method::Get {
                *get_count2.lock().unwrap() += 1;
                if file_path.is_file() {
                    let data = fs::read(&file_path).unwrap_or_default();
                    let _ = request.respond(Response::from_data(data));
                } else {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
            } else {
                let _ = request.respond(Response::empty(StatusCode(405)));
            }
        }
    });
    thread::sleep(Duration::from_millis(20));

    run_ok(&[
        "doctor",
        "--source",
        &base,
        "--no-probe",
        idx.to_str().unwrap(),
    ]);

    let heads = *head_count.lock().unwrap();
    let gets = *get_count.lock().unwrap();
    assert!(
        heads >= 1,
        "doctor default path must probe via has (HEAD); heads={heads} gets={gets}"
    );
    assert_eq!(
        gets, 0,
        "doctor without --deep must not GET chunk bodies; heads={heads} gets={gets}"
    );
}

#[test]
fn doctor_http_missing_chunk_via_has() {
    let dir = tempdir().unwrap();
    let remote_store = dir.path().join("remote");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        remote_store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let (cnk_path, missing_id) = first_cnk_id(&remote_store.join("chunks"));
    fs::remove_file(&cnk_path).unwrap();

    let (base, _handle) = spawn_static_store_server(remote_store);

    let out = run_fail(&[
        "doctor",
        "--source",
        &base,
        "--no-probe",
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.lines().any(|l| l.trim() == missing_id),
        "stdout must contain missing id {missing_id}, got:\n{stdout}"
    );
}

#[test]
fn doctor_deep_flag_uses_get() {
    let dir = tempdir().unwrap();
    let remote_store = dir.path().join("remote");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        remote_store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let get_count = Arc::new(Mutex::new(0usize));
    let get_count2 = Arc::clone(&get_count);
    let store_root = remote_store.clone();

    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let _handle = thread::spawn(move || {
        for request in server.incoming_requests() {
            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(&url);
            let rel = path.trim_start_matches('/');
            let file_path = store_root.join(rel);

            if request.method() == &Method::Get {
                *get_count2.lock().unwrap() += 1;
                if file_path.is_file() {
                    let data = fs::read(&file_path).unwrap_or_default();
                    let _ = request.respond(Response::from_data(data));
                } else {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
            } else if request.method() == &Method::Head {
                // has() path unused when --deep; still answer.
                if file_path.is_file() {
                    let _ = request.respond(Response::empty(200));
                } else {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
            } else {
                let _ = request.respond(Response::empty(StatusCode(405)));
            }
        }
    });
    thread::sleep(Duration::from_millis(20));

    run_ok(&[
        "doctor",
        "--source",
        &base,
        "--deep",
        "--no-probe",
        idx.to_str().unwrap(),
    ]);

    let gets = *get_count.lock().unwrap();
    assert!(gets >= 1, "doctor --deep must use get; got gets={gets}");
}

// --- Phase 3 M5: gc (local dry-run / --apply) ---

#[test]
fn gc_help_lists_flags() {
    let help = run_ok(&["--help"]);
    let help_s = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_s.contains("gc"),
        "top-level help should list gc:\n{help_s}"
    );

    let g = run_ok(&["gc", "--help"]);
    let s = String::from_utf8_lossy(&g.stdout);
    assert!(s.contains("--store"), "{s}");
    assert!(s.contains("--apply"), "{s}");
    assert!(s.contains("--jobs"), "{s}");
}

#[test]
fn gc_dry_run_lists_loose_chunk_outside_two_indexes() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx_a = dir.path().join("a.cfidx");
    let idx_b = dir.path().join("b.cfidx");
    let input_a = fixtures_dir().join("hello.txt");
    let input_b = fixtures_dir().join("binary-256.bin");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx_a.to_str().unwrap(),
        input_a.to_str().unwrap(),
    ]);
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx_b.to_str().unwrap(),
        input_b.to_str().unwrap(),
    ]);

    // Inject a loose orphan chunk not referenced by either index.
    let orphan_plain = b"orphan-loose-chunk-for-gc-m5";
    let orphan_id = chunkforge_store::ChunkId::hash(orphan_plain);
    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        s.put(orphan_plain).unwrap();
        assert!(s.has(&orphan_id));
    }

    let out = run_ok(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        idx_a.to_str().unwrap(),
        idx_b.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let orphan_path = store
        .join("chunks")
        .join(&orphan_id.to_hex()[0..2])
        .join(format!("{}.cnk", &orphan_id.to_hex()[2..]));
    assert!(
        stdout.lines().any(|l| {
            let t = l.trim();
            t == orphan_path.to_str().unwrap()
                || t.ends_with(orphan_path.file_name().unwrap().to_str().unwrap())
                    && t.contains(&orphan_id.to_hex()[0..2])
        }),
        "dry-run stdout must list orphan path {}; got:\n{stdout}",
        orphan_path.display()
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("dry-run") || err.contains("unreferenced"),
        "stderr={err}"
    );

    // Dry-run must not delete.
    assert!(orphan_path.is_file(), "dry-run must leave orphan on disk");
    let has = run_ok(&[
        "store",
        "has",
        "--store",
        store.to_str().unwrap(),
        &orphan_id.to_hex(),
    ]);
    assert!(
        String::from_utf8_lossy(&has.stdout).contains("present"),
        "orphan must still be present after dry-run"
    );
}

#[test]
fn gc_apply_deletes_orphan_keeps_referenced() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx_a = dir.path().join("a.cfidx");
    let idx_b = dir.path().join("b.cfidx");
    let input_a = fixtures_dir().join("hello.txt");
    let input_b = fixtures_dir().join("binary-256.bin");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx_a.to_str().unwrap(),
        input_a.to_str().unwrap(),
    ]);
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx_b.to_str().unwrap(),
        input_b.to_str().unwrap(),
    ]);

    // Collect a referenced id from index A before injecting orphan.
    let (ref_path, ref_id) = first_cnk_id(&store.join("chunks"));

    let orphan_plain = b"orphan-to-delete-via-apply";
    let orphan_id = chunkforge_store::ChunkId::hash(orphan_plain);
    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        s.put(orphan_plain).unwrap();
        assert!(s.has(&orphan_id));
    }

    run_ok(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        "--apply",
        idx_a.to_str().unwrap(),
        idx_b.to_str().unwrap(),
    ]);

    // Orphan gone.
    let missing = run_fail(&[
        "store",
        "has",
        "--store",
        store.to_str().unwrap(),
        &orphan_id.to_hex(),
    ]);
    let miss_err = String::from_utf8_lossy(&missing.stderr);
    assert!(
        miss_err.contains("missing") || miss_err.contains(&orphan_id.to_hex()),
        "stderr={miss_err}"
    );

    // Referenced chunk from before orphan injection still present.
    assert!(
        ref_path.is_file(),
        "referenced chunk {} must remain",
        ref_path.display()
    );
    run_ok(&["store", "has", "--store", store.to_str().unwrap(), &ref_id]);

    // Indexes still verify cleanly.
    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        idx_a.to_str().unwrap(),
    ]);
    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        idx_b.to_str().unwrap(),
    ]);
}

#[test]
fn gc_dry_run_clean_store_prints_nothing() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    assert!(
        out.stdout.is_empty(),
        "clean store dry-run should print no paths; got {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("nothing to reclaim") || err.contains("gc:"),
        "stderr={err}"
    );
}

// --- Phase 12 M1: gc --jobs ---

#[test]
fn gc_jobs_help_and_zero_rejected() {
    let g = run_ok(&["gc", "--help"]);
    let s = String::from_utf8_lossy(&g.stdout);
    assert!(s.contains("--jobs"), "gc --help must list --jobs:\n{s}");

    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let out = run_fail(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "0",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("jobs") && (err.contains(">= 1") || err.contains("0")),
        "stderr={err}"
    );
}

#[test]
fn gc_jobs_dry_run_path_set_matches_serial() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("a.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // Several orphans so path-set equality is meaningful.
    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        for plain in [
            &b"orphan-gc-jobs-a"[..],
            &b"orphan-gc-jobs-b"[..],
            &b"orphan-gc-jobs-c"[..],
        ] {
            s.put(plain).unwrap();
        }
    }

    let out1 = run_ok(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "1",
        idx.to_str().unwrap(),
    ]);
    let out4 = run_ok(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "4",
        idx.to_str().unwrap(),
    ]);
    // Default (no --jobs) ≡ jobs=1
    let out_default = run_ok(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);

    let mut paths1: Vec<String> = String::from_utf8_lossy(&out1.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let mut paths4: Vec<String> = String::from_utf8_lossy(&out4.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let mut paths_def: Vec<String> = String::from_utf8_lossy(&out_default.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    paths1.sort();
    paths4.sort();
    paths_def.sort();
    assert_eq!(paths1, paths4, "jobs=1 vs jobs=4 dry-run path sets");
    assert_eq!(paths1, paths_def, "default (no --jobs) ≡ jobs=1 path set");
    assert_eq!(paths1.len(), 3, "expected 3 orphan paths; got {paths1:?}");

    // Dry-run must not delete.
    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        for plain in [
            &b"orphan-gc-jobs-a"[..],
            &b"orphan-gc-jobs-b"[..],
            &b"orphan-gc-jobs-c"[..],
        ] {
            let id = chunkforge_store::ChunkId::hash(plain);
            assert!(s.has(&id), "orphan {} must remain after dry-run", id);
        }
    }
}

#[test]
fn gc_apply_jobs_four_deletes_orphan() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("a.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let orphan_plain = b"orphan-gc-apply-jobs-4";
    let orphan_id = chunkforge_store::ChunkId::hash(orphan_plain);
    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        s.put(orphan_plain).unwrap();
        assert!(s.has(&orphan_id));
    }

    run_ok(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "4",
        "--apply",
        idx.to_str().unwrap(),
    ]);

    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        assert!(
            !s.has(&orphan_id),
            "orphan must be gone after --apply --jobs 4"
        );
    }
    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
}

// --- Phase 12 M2: gc --format text|json ---

#[test]
fn gc_help_lists_format() {
    let g = run_ok(&["gc", "--help"]);
    let s = String::from_utf8_lossy(&g.stdout);
    assert!(s.contains("--format"), "gc --help must list --format:\n{s}");
}

#[test]
fn gc_format_json_dry_run_parseable() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("a.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let orphan_plain = b"orphan-gc-format-json-dry";
    let orphan_id = chunkforge_store::ChunkId::hash(orphan_plain);
    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        s.put(orphan_plain).unwrap();
        assert!(s.has(&orphan_id));
    }

    let out = run_ok(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("gc: dry-run") && !stderr.contains("nothing to reclaim"),
        "json must not duplicate text stderr summary; stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("gc json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["applied"], false);
    assert_eq!(v["listings"].as_u64(), Some(1));
    assert!(v["referenced"].as_u64().unwrap() >= 1, "referenced={v}");
    assert_eq!(v["unreferenced"].as_u64(), Some(1));
    assert_eq!(v["deleted"].as_u64(), Some(0));
    // No path lines — sole stdout payload is the JSON object.
    assert!(
        !stdout.contains(".cnk"),
        "json stdout must not list .cnk paths; got {stdout}"
    );

    // Dry-run must not delete.
    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        assert!(s.has(&orphan_id), "orphan must remain after json dry-run");
    }
}

#[test]
fn gc_format_json_apply_deleted() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("a.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let orphan_plain = b"orphan-gc-format-json-apply";
    let orphan_id = chunkforge_store::ChunkId::hash(orphan_plain);
    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        s.put(orphan_plain).unwrap();
        assert!(s.has(&orphan_id));
    }

    let out = run_ok(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        "--apply",
        "--format",
        "json",
        "--jobs",
        "2",
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("gc: deleted"),
        "json must not duplicate text stderr summary; stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("gc apply json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["dry_run"], false);
    assert_eq!(v["applied"], true);
    assert_eq!(v["listings"].as_u64(), Some(1));
    assert_eq!(v["unreferenced"].as_u64(), Some(1));
    assert_eq!(v["deleted"].as_u64(), Some(1));

    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        assert!(
            !s.has(&orphan_id),
            "orphan must be gone after --apply --format json"
        );
    }
}

#[test]
fn gc_default_format_is_text_not_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("a.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        s.put(b"orphan-gc-default-text").unwrap();
    }

    let out = run_ok(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Default ≡ text: path(s) on stdout, summary on stderr — not a pure JSON object.
    assert!(
        stdout.contains(".cnk") || stdout.lines().any(|l| !l.trim().is_empty()),
        "default text should list paths on stdout; got {stdout:?}"
    );
    assert!(
        stderr.contains("dry-run") || stderr.contains("unreferenced"),
        "default text should keep stderr summary; stderr={stderr}"
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(stdout.trim()).is_err(),
        "default (no --format) stdout must not be pure JSON; got {stdout}"
    );
}

#[test]
fn gc_format_json_nothing_to_reclaim() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "gc",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("gc clean json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["applied"], false);
    assert_eq!(v["unreferenced"].as_u64(), Some(0));
    assert_eq!(v["deleted"].as_u64(), Some(0));
    assert!(v["referenced"].as_u64().unwrap() >= 1);
}

// --- Phase 4 M3: push (serial) + dry-run ---

use std::sync::atomic::{AtomicUsize, Ordering};

/// PUT/HEAD/GET stub that mirrors the default CAS layout under `store_root`
/// (`chunks/<2hex>/<62hex>.cnk`). Counts PUT bodies for dry-run assertions.
fn spawn_put_get_store_server(
    store_root: PathBuf,
    put_count: Arc<AtomicUsize>,
) -> (String, thread::JoinHandle<()>) {
    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let handle = thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let method = request.method().clone();
            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(&url);
            let rel = path.trim_start_matches('/');
            let file_path = store_root.join(rel);

            match method {
                Method::Head => {
                    if file_path.is_file() {
                        let len = fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
                        let response = Response::empty(200).with_header(
                            Header::from_bytes(&b"Content-Length"[..], len.to_string()).unwrap(),
                        );
                        let _ = request.respond(response);
                    } else {
                        let _ = request.respond(Response::empty(StatusCode(404)));
                    }
                }
                Method::Get => {
                    if file_path.is_file() {
                        let data = fs::read(&file_path).unwrap_or_default();
                        let _ = request.respond(Response::from_data(data));
                    } else {
                        let _ = request.respond(Response::empty(StatusCode(404)));
                    }
                }
                Method::Put | Method::Post => {
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                    put_count.fetch_add(1, Ordering::SeqCst);
                    if let Some(parent) = file_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(&file_path, &body);
                    let _ = request.respond(
                        Response::empty(StatusCode(200))
                            .with_header(Header::from_bytes(&b"Content-Length"[..], "0").unwrap()),
                    );
                }
                _ => {
                    let _ = request.respond(Response::empty(StatusCode(405)));
                }
            }
        }
    });
    thread::sleep(Duration::from_millis(20));
    (base, handle)
}

#[test]
fn push_help_lists_store_dest_dry_run_templates() {
    let help = run_ok(&["--help"]);
    let top = String::from_utf8_lossy(&help.stdout);
    assert!(
        top.contains("push"),
        "top-level help should list push:\n{top}"
    );

    let p = run_ok(&["push", "--help"]);
    let s = String::from_utf8_lossy(&p.stdout);
    assert!(s.contains("--store"), "{s}");
    assert!(s.contains("--dest"), "{s}");
    assert!(s.contains("--dry-run"), "{s}");
    assert!(s.contains("--verify"), "{s}");
    assert!(s.contains("--url-template"), "{s}");
    assert!(s.contains("--prefix"), "{s}");
    assert!(s.contains("--header"), "{s}");
    assert!(
        s.contains("--http-retries"),
        "push --help should list --http-retries:\n{s}"
    );
    assert!(
        s.to_ascii_lowercase().contains("cfidx"),
        "help should mention .cfidx:\n{s}"
    );
    assert!(
        s.to_ascii_lowercase().contains("cfdir"),
        "help should mention .cfdir:\n{s}"
    );
    let lower = s.to_ascii_lowercase();
    // Phase20-M6: --dest may be http(s) / local path / file:// (not HTTP-only).
    assert!(
        lower.contains("file://") && lower.contains("local"),
        "push --help should mention local and file:// dest:\n{s}"
    );
}

#[test]
fn push_local_dest_rejects_http_template_flags() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let dest = dir.path().join("dest-store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_fail(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        dest.to_str().unwrap(),
        "--url-template",
        "{base}/{path}",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("url-template")
            || err.contains("prefix")
            || err.contains("header")
            || err.contains("http"),
        "local dest + --url-template must fail clearly; stderr={err}"
    );
    assert!(
        err.contains("dest") || err.contains("local") || err.contains("file"),
        "error should name dest/local; stderr={err}"
    );
}

#[test]
fn push_local_dest_store_smoke_with_verify() {
    let dir = tempdir().unwrap();
    let tree = dir.path().join("tree");
    fs::create_dir_all(&tree).unwrap();
    fs::write(tree.join("a.txt"), b"phase20-m6-local-dest-smoke").unwrap();
    let store = dir.path().join("store");
    let dest = dir.path().join("dest-store");
    let listing = dir.path().join("out.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        tree.to_str().unwrap(),
    ]);
    assert!(count_cnk(&store) >= 1, "source store should have chunks");

    let out = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        dest.to_str().unwrap(),
        "--verify",
        listing.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("uploaded=") || err.contains("skipped="),
        "push summary missing; stderr={err}"
    );
    assert!(
        dest.join("meta.toml").is_file(),
        "dest store meta.toml missing at {}",
        dest.display()
    );
    assert!(
        count_cnk(&dest) >= 1,
        "dest store should contain pushed .cnk files"
    );
    assert!(
        err.to_ascii_lowercase().contains("verify ok")
            || err.to_ascii_lowercase().contains("verify"),
        "push --verify should report verify; stderr={err}"
    );
}

#[test]
fn push_file_url_dest_smoke() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let dest = dir.path().join("dest-file-url");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let dest_url = format!("file://{}", dest.display());
    let out = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &dest_url,
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("uploaded=") || err.contains("skipped="),
        "push file:// summary missing; stderr={err}"
    );
    assert!(dest.join("meta.toml").is_file(), "file:// dest store missing");
    assert!(count_cnk(&dest) >= 1, "file:// dest should have .cnk files");
}

#[test]
fn push_to_mock_then_verify_source_succeeds() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    let out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("uploaded="), "stderr={err}");
    assert!(
        put_count.load(Ordering::SeqCst) >= 1,
        "expected at least one PUT, got {}",
        put_count.load(Ordering::SeqCst)
    );
    assert!(
        count_cnk(&mirror.join("chunks")) >= 1,
        "mirror should contain uploaded .cnk files"
    );

    run_ok(&["verify", "--source", &base, idx.to_str().unwrap()]);
}

#[test]
fn push_dry_run_issues_no_put() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    let out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        "--dry-run",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("dry_run=true") || err.contains("uploaded="),
        "stderr={err}"
    );
    assert_eq!(
        put_count.load(Ordering::SeqCst),
        0,
        "dry-run must not issue PUT"
    );
    assert_eq!(
        count_cnk(&mirror.join("chunks")),
        0,
        "dry-run must not write remote objects"
    );
}

#[test]
fn push_second_run_idempotent_uploaded_zero() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));

    run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        idx.to_str().unwrap(),
    ]);
    let first_puts = put_count.load(Ordering::SeqCst);
    assert!(first_puts >= 1, "first push should PUT, got {first_puts}");

    let out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("uploaded=0"),
        "second push should report uploaded=0; stderr={err}"
    );
    assert!(
        err.contains("skipped=") && !err.contains("skipped=0"),
        "second push should skip existing chunks; stderr={err}"
    );
    assert_eq!(
        put_count.load(Ordering::SeqCst),
        first_puts,
        "idempotent push must not issue additional PUTs"
    );
}

#[test]
fn push_missing_local_chunk_fails() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    delete_cnk_files(&local.join("chunks"));

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));

    let out = run_fail(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("fail") || err.contains("unavailable") || err.contains("missing"),
        "stderr={err}"
    );
    assert_eq!(put_count.load(Ordering::SeqCst), 0);
}

// --- Phase 4 M5: bounded concurrency `--jobs` ---

#[test]
fn jobs_help_listed_on_cat_verify_doctor_push() {
    for cmd in ["cat", "verify", "doctor", "push", "extract", "pull"] {
        let out = run_ok(&[cmd, "--help"]);
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(
            s.contains("--jobs"),
            "{cmd} --help should list --jobs:\n{s}"
        );
    }
}

#[test]
fn jobs_zero_rejected() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let out = run_fail(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "0",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("jobs") && (err.contains(">= 1") || err.contains("0")),
        "stderr={err}"
    );
}

/// Multi-chunk local verify: `--jobs 1` and `--jobs 4` both succeed; cat bytes match.
#[test]
fn verify_jobs_four_matches_jobs_one_bytes() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("multi.cfidx");
    let out1 = dir.path().join("out1.bin");
    let out4 = dir.path().join("out4.bin");

    // Patterned multi-chunk blob (same recipe as dedup mid-file test).
    let mut data = Vec::with_capacity(48 * 1024);
    for i in 0..(48 * 1024) {
        data.push(((i * 17 + 3) % 251) as u8);
    }
    let input = dir.path().join("multi.bin");
    fs::write(&input, &data).unwrap();
    let chunk_size = "2048:4096:8192";

    let make_out = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--chunk-size",
        chunk_size,
        input.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&make_out.stderr);
    let (new_chunks, _) = parse_make_stats(&err);
    assert!(
        new_chunks >= 4,
        "need several chunks for jobs coverage, got new={new_chunks} ({err})"
    );

    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "1",
        idx.to_str().unwrap(),
    ]);
    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "4",
        idx.to_str().unwrap(),
    ]);

    run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "1",
        idx.to_str().unwrap(),
        "-o",
        out1.to_str().unwrap(),
    ]);
    run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "4",
        idx.to_str().unwrap(),
        "-o",
        out4.to_str().unwrap(),
    ]);

    let a = fs::read(&out1).unwrap();
    let b = fs::read(&out4).unwrap();
    assert_eq!(a, data, "jobs=1 cat must match original");
    assert_eq!(b, data, "jobs=4 cat must match original");
    assert_eq!(a, b, "jobs=1 and jobs=4 cat must be byte-identical");
}

#[test]
fn verify_jobs_four_http_source_against_mock() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("multi.cfidx");

    let mut data = Vec::with_capacity(32 * 1024);
    for i in 0..(32 * 1024) {
        data.push(((i * 31 + 7) % 251) as u8);
    }
    let input = dir.path().join("multi.bin");
    fs::write(&input, &data).unwrap();

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--chunk-size",
        "2048:4096:8192",
        input.to_str().unwrap(),
    ]);

    let (base, _handle) = spawn_static_store_server(store.clone());

    run_ok(&[
        "verify",
        "--source",
        &base,
        "--jobs",
        "4",
        idx.to_str().unwrap(),
    ]);
    // Default jobs=1 still works against the same mock.
    run_ok(&["verify", "--source", &base, idx.to_str().unwrap()]);
}

#[test]
fn verify_jobs_missing_chunk_error_includes_id() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("multi.cfidx");

    let mut data = Vec::with_capacity(24 * 1024);
    for i in 0..(24 * 1024) {
        data.push(((i * 13 + 5) % 251) as u8);
    }
    let input = dir.path().join("multi.bin");
    fs::write(&input, &data).unwrap();

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--chunk-size",
        "2048:4096:8192",
        input.to_str().unwrap(),
    ]);

    // Collect one chunk id, delete its .cnk, then verify --jobs 4.
    let chunk_id_out = run_ok(&[
        "chunk-id",
        "--chunk-size",
        "2048:4096:8192",
        input.to_str().unwrap(),
    ]);
    let first_line = String::from_utf8_lossy(&chunk_id_out.stdout)
        .lines()
        .next()
        .expect("chunk line")
        .to_string();
    let hex_id = first_line.split('\t').nth(2).expect("id column");
    assert_eq!(hex_id.len(), 64);
    let cnk = store
        .join("chunks")
        .join(&hex_id[..2])
        .join(format!("{}.cnk", &hex_id[2..]));
    assert!(cnk.is_file(), "expected {}", cnk.display());
    fs::remove_file(&cnk).unwrap();

    let out = run_fail(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "4",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains(hex_id),
        "error must include chunk id {hex_id}; stderr={err}"
    );
}

#[test]
fn doctor_jobs_four_ok_and_missing() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("multi.cfidx");

    let mut data = Vec::with_capacity(24 * 1024);
    for i in 0..(24 * 1024) {
        data.push(((i * 19 + 11) % 251) as u8);
    }
    let input = dir.path().join("multi.bin");
    fs::write(&input, &data).unwrap();

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--chunk-size",
        "2048:4096:8192",
        input.to_str().unwrap(),
    ]);

    let ok = run_ok(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "4",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&ok.stderr);
    assert!(err.contains("doctor: ok"), "stderr={err}");

    delete_cnk_files(&store.join("chunks"));
    let fail = run_fail(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "4",
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&fail.stdout);
    assert!(
        stdout.lines().any(|l| l.len() == 64),
        "missing ids on stdout; got={stdout}"
    );
}

#[test]
fn push_jobs_four_to_mock_then_verify() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("multi.cfidx");

    let mut data = Vec::with_capacity(24 * 1024);
    for i in 0..(24 * 1024) {
        data.push(((i * 23 + 1) % 251) as u8);
    }
    let input = dir.path().join("multi.bin");
    fs::write(&input, &data).unwrap();

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--chunk-size",
        "2048:4096:8192",
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    let out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        "--jobs",
        "4",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("uploaded="), "stderr={err}");
    let puts = put_count.load(Ordering::SeqCst);
    assert!(
        puts >= 2,
        "expected multiple PUTs under --jobs 4, got {puts}; stderr={err}"
    );
    assert!(
        err.contains("failed=0"),
        "push --jobs 4 should report failed=0; stderr={err}"
    );

    run_ok(&[
        "verify",
        "--source",
        &base,
        "--jobs",
        "4",
        idx.to_str().unwrap(),
    ]);
}

#[test]
fn push_jobs_one_matches_serial_stats_shape() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));

    let out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        "--jobs",
        "1",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("uploaded="), "stderr={err}");
    assert!(err.contains("skipped="), "stderr={err}");
    assert!(err.contains("failed=0"), "stderr={err}");
    assert!(put_count.load(Ordering::SeqCst) >= 1);
}

// --- Phase 8 M2: CLI `--http-retries` ---

/// PUT/HEAD stub: first `fail_puts` successful-path PUTs return 503; afterwards
/// behave like [`spawn_put_get_store_server`]. HEAD always reflects on-disk state
/// (missing → 404). Counts every PUT attempt (including 503s).
fn spawn_flaky_put_store_server(
    store_root: PathBuf,
    fail_puts: usize,
    put_attempts: Arc<AtomicUsize>,
) -> (String, thread::JoinHandle<()>) {
    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let fail_left = Arc::new(AtomicUsize::new(fail_puts));
    let handle = thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let method = request.method().clone();
            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(&url);
            let rel = path.trim_start_matches('/');
            let file_path = store_root.join(rel);

            match method {
                Method::Head => {
                    if file_path.is_file() {
                        let len = fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
                        let response = Response::empty(200).with_header(
                            Header::from_bytes(&b"Content-Length"[..], len.to_string()).unwrap(),
                        );
                        let _ = request.respond(response);
                    } else {
                        let _ = request.respond(Response::empty(StatusCode(404)));
                    }
                }
                Method::Get => {
                    if file_path.is_file() {
                        let data = fs::read(&file_path).unwrap_or_default();
                        let _ = request.respond(Response::from_data(data));
                    } else {
                        let _ = request.respond(Response::empty(StatusCode(404)));
                    }
                }
                Method::Put | Method::Post => {
                    put_attempts.fetch_add(1, Ordering::SeqCst);
                    // Consume body even on 503 so the client does not hang.
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                    let remaining = fail_left.load(Ordering::SeqCst);
                    if remaining > 0 {
                        fail_left.fetch_sub(1, Ordering::SeqCst);
                        let _ = request.respond(Response::empty(StatusCode(503)));
                        continue;
                    }
                    if let Some(parent) = file_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(&file_path, &body);
                    let _ = request.respond(
                        Response::empty(StatusCode(200))
                            .with_header(Header::from_bytes(&b"Content-Length"[..], "0").unwrap()),
                    );
                }
                _ => {
                    let _ = request.respond(Response::empty(StatusCode(405)));
                }
            }
        }
    });
    thread::sleep(Duration::from_millis(20));
    (base, handle)
}

#[test]
fn http_retries_help_listed_on_http_commands() {
    for cmd in ["cat", "verify", "doctor", "push", "extract", "pull"] {
        let out = run_ok(&[cmd, "--help"]);
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(
            s.contains("--http-retries"),
            "{cmd} --help should list --http-retries:\n{s}"
        );
        assert!(
            s.contains("--http-retry-backoff-ms"),
            "{cmd} --help should list --http-retry-backoff-ms:\n{s}"
        );
    }
}

/// PUT/HEAD stub that always responds with a fixed status (body discarded).
fn spawn_fixed_status_put_server(status: u16) -> (String, thread::JoinHandle<()>) {
    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let handle = thread::spawn(move || {
        for mut request in server.incoming_requests() {
            // Consume PUT/POST body so the client does not hang.
            match request.method() {
                Method::Put | Method::Post => {
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                }
                _ => {}
            }
            let _ = request.respond(Response::empty(StatusCode(status)));
        }
    });
    thread::sleep(Duration::from_millis(20));
    (base, handle)
}

/// Phase8-M2: stub returns 503 then succeeds — `--http-retries 0` fails;
/// `--http-retries 3` succeeds and summary includes `retries=3`.
#[test]
fn push_http_retries_zero_fails_three_succeeds_on_transient_503() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // --- retries=0: first PUT is 503 → push fails after one attempt ---
    let attempts0 = Arc::new(AtomicUsize::new(0));
    let (base0, _h0) = spawn_flaky_put_store_server(mirror.clone(), 1, Arc::clone(&attempts0));
    let fail = run_fail(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base0,
        "--http-retries",
        "0",
        "--http-retry-backoff-ms",
        "0",
        idx.to_str().unwrap(),
    ]);
    let fail_err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        fail_err.contains("failed_transient=")
            && !fail_err.contains("failed=0 ")
            && !fail_err.contains("failed_transient=0"),
        "retries=0 should fail on 503 as transient; stderr={fail_err}"
    );
    assert!(
        fail_err.contains("retries=0"),
        "summary should include retries=0; stderr={fail_err}"
    );
    assert_eq!(
        attempts0.load(Ordering::SeqCst),
        1,
        "max_retries=0 → exactly one PUT attempt"
    );

    // --- retries=3: two 503s then 200 → success ---
    let mirror2 = dir.path().join("mirror2");
    fs::create_dir_all(&mirror2).unwrap();
    let attempts3 = Arc::new(AtomicUsize::new(0));
    let (base3, _h3) = spawn_flaky_put_store_server(mirror2, 2, Arc::clone(&attempts3));
    let ok = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base3,
        "--http-retries",
        "3",
        "--http-retry-backoff-ms",
        "0",
        idx.to_str().unwrap(),
    ]);
    let ok_err = String::from_utf8_lossy(&ok.stderr);
    assert!(
        ok_err.contains("failed=0 ")
            && ok_err.contains("failed_transient=0")
            && ok_err.contains("failed_permanent=0"),
        "retries=3 should succeed after transient 503s; stderr={ok_err}"
    );
    assert!(
        ok_err.contains("retries=3"),
        "summary should include retries=3; stderr={ok_err}"
    );
    assert!(
        attempts3.load(Ordering::SeqCst) >= 3,
        "expected ≥3 PUT attempts (2×503 + 1×200), got {}",
        attempts3.load(Ordering::SeqCst)
    );

    // Post-push verify against the flaky-then-stable mirror (now has objects).
    run_ok(&[
        "verify",
        "--source",
        &base3,
        "--http-retries",
        "0",
        idx.to_str().unwrap(),
    ]);
}

/// Phase8-M3: 401 → failed_permanent; 503 → failed_transient (distinguishable).
#[test]
fn push_summary_distinguishes_401_permanent_vs_503_transient() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // --- 401 permanent ---
    let (base401, _h401) = spawn_fixed_status_put_server(401);
    let fail401 = run_fail(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base401,
        "--http-retries",
        "3",
        "--http-retry-backoff-ms",
        "0",
        idx.to_str().unwrap(),
    ]);
    let err401 = String::from_utf8_lossy(&fail401.stderr);
    assert!(
        err401.contains("401"),
        "error path should mention 401; stderr={err401}"
    );
    assert!(
        err401.contains("failed_permanent=") && !err401.contains("failed_permanent=0"),
        "401 must count as failed_permanent; stderr={err401}"
    );
    assert!(
        err401.contains("failed_transient=0"),
        "401 must not count as transient; stderr={err401}"
    );

    // --- 503 transient ---
    let (base503, _h503) = spawn_fixed_status_put_server(503);
    let fail503 = run_fail(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base503,
        "--http-retries",
        "0",
        "--http-retry-backoff-ms",
        "0",
        idx.to_str().unwrap(),
    ]);
    let err503 = String::from_utf8_lossy(&fail503.stderr);
    assert!(
        err503.contains("503"),
        "error path should mention 503; stderr={err503}"
    );
    assert!(
        err503.contains("failed_transient=") && !err503.contains("failed_transient=0"),
        "503 must count as failed_transient; stderr={err503}"
    );
    assert!(
        err503.contains("failed_permanent=0"),
        "503 must not count as permanent; stderr={err503}"
    );
}

/// Local `--store` path ignores `--http-retries` (no-op; behaviour unchanged).
#[test]
fn verify_local_store_ignores_http_retries_flag() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");
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
        "--http-retries",
        "3",
        "--http-retry-backoff-ms",
        "0",
        idx.to_str().unwrap(),
    ]);
}

#[test]
fn archive_help_documents_symlink_policy() {
    let help = run_ok(&["archive", "--help"]);
    let help_s = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_s.contains("symlink") || help_s.contains("Symlink"),
        "archive --help should mention symlink policy; got:\n{help_s}"
    );
    assert!(help_s.contains("--store"), "{help_s}");
    assert!(
        help_s.contains("--chunk-size") || help_s.contains("chunk-size"),
        "{help_s}"
    );
}

#[test]
fn archive_small_tree_writes_chunks_and_second_run_reuses() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    let sub = src.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(src.join("a.txt"), b"hello-tree\n").unwrap();
    fs::write(sub.join("b.txt"), b"hello-tree\n").unwrap(); // cross-file dedup
    fs::write(sub.join("c.bin"), b"unique-payload-xyz").unwrap();

    // Symlink + fifo should be skipped (warn), not fail the archive.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink("a.txt", src.join("link-to-a")).unwrap();
        // Best-effort fifo; ignore if mkfifo unavailable.
        let fifo = src.join("my.fifo");
        let _ = Command::new("mkfifo").arg(&fifo).status();
    }

    let store = dir.path().join("store");
    let out1 = dir.path().join("rel1.cfdir");
    let out2 = dir.path().join("rel2.cfdir");

    let first = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err1 = String::from_utf8_lossy(&first.stderr);
    assert!(err1.contains("archive: wrote"), "stderr={err1}");
    assert!(err1.contains("new="), "stderr={err1}");
    assert!(err1.contains("reused="), "stderr={err1}");
    assert!(out1.is_file(), "missing {}", out1.display());

    // Decode .cfdir: expect 3 regular files (symlink/fifo skipped).
    let bytes = fs::read(&out1).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).expect("decode .cfdir");
    assert_eq!(
        arch.entries.len(),
        3,
        "entries={:?}",
        arch.entries.iter().map(|e| &e.path).collect::<Vec<_>>()
    );
    let paths: Vec<_> = arch.entries.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"a.txt"), "{paths:?}");
    assert!(paths.contains(&"sub/b.txt"), "{paths:?}");
    assert!(paths.contains(&"sub/c.bin"), "{paths:?}");

    // Store must contain chunks.
    let store_h = chunkforge_store::Store::open(&store).expect("open store");
    let listed = store_h.list_chunk_ids().expect("list chunks");
    assert!(
        !listed.is_empty(),
        "store should have chunks after archive; stderr={err1}"
    );
    let chunk_count_before = listed.len();

    // Second archive of same tree → new≈0 (content unchanged).
    let second = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out2.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err2 = String::from_utf8_lossy(&second.stderr);
    assert!(
        err2.contains("new=0"),
        "second archive should reuse all chunks; stderr={err2}"
    );
    let listed2 = store_h.list_chunk_ids().expect("list chunks again");
    assert_eq!(
        listed2.len(),
        chunk_count_before,
        "store chunk count must not grow on identical re-archive"
    );

    #[cfg(unix)]
    {
        assert!(
            err1.contains("symlink") || err1.contains("skip"),
            "expected symlink skip warning; stderr={err1}"
        );
    }
}

#[test]
fn archive_empty_dir_writes_empty_cfdir() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("empty-src");
    fs::create_dir_all(&src).unwrap();
    let store = dir.path().join("store");
    let out = dir.path().join("empty.cfdir");
    let result = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&result.stderr);
    assert!(err.contains("0 file"), "stderr={err}");
    let bytes = fs::read(&out).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).unwrap();
    assert!(arch.entries.is_empty());
}

#[test]
fn make_single_file_unchanged_alongside_archive() {
    // Regression: make still produces .cfidx for a single file.
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let bytes = fs::read(&idx).unwrap();
    assert_eq!(&bytes[0..5], b"CFIDX");
    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
}

// --- Phase 5 M3: extract + verify .cfdir ---

#[test]
fn extract_help_lists_store_source_output() {
    let help = run_ok(&["extract", "--help"]);
    let help_s = String::from_utf8_lossy(&help.stdout);
    assert!(help_s.contains("--store"), "{help_s}");
    assert!(help_s.contains("--source"), "{help_s}");
    assert!(
        help_s.contains("-o") || help_s.contains("--output"),
        "{help_s}"
    );
    assert!(help_s.contains("--jobs"), "{help_s}");
}

#[test]
fn archive_extract_roundtrip_diff_qr() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    let sub = src.join("sub");
    let nested = sub.join("deep");
    fs::create_dir_all(&nested).unwrap();
    fs::write(src.join("a.txt"), b"hello-tree\n").unwrap();
    fs::write(sub.join("b.txt"), b"hello-tree\n").unwrap(); // cross-file dedup
    fs::write(nested.join("c.bin"), b"unique-payload-xyz").unwrap();
    fs::write(src.join("empty-file"), b"").unwrap();

    let store = dir.path().join("store");
    let cfdir = dir.path().join("release.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    // verify .cfdir before extract
    let ver = run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);
    let ver_err = String::from_utf8_lossy(&ver.stderr);
    assert!(ver_err.contains("verify: ok"), "stderr={ver_err}");
    assert!(ver_err.contains("file"), "stderr={ver_err}");

    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);

    // Content-equivalent to source for regular files (diff -qr).
    let diff = Command::new("diff")
        .args(["-qr", src.to_str().unwrap(), out.to_str().unwrap()])
        .output()
        .expect("spawn diff");
    assert!(
        diff.status.success(),
        "diff -qr failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&diff.stdout),
        String::from_utf8_lossy(&diff.stderr)
    );

    // Existing target must fail (no --force).
    let fail = run_fail(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    let fail_err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        fail_err.contains("already exists") || fail_err.contains("refusing"),
        "stderr={fail_err}"
    );
}

#[test]
fn verify_cfdir_missing_chunk_includes_id() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    // Multi-chunk file so deleting one .cnk is meaningful.
    let mut data = Vec::with_capacity(24 * 1024);
    for i in 0..(24 * 1024) {
        data.push(((i * 13 + 5) % 251) as u8);
    }
    fs::write(src.join("big.bin"), &data).unwrap();
    fs::write(src.join("sub/small.txt"), b"tiny\n").unwrap();

    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        "--chunk-size",
        "2048:4096:8192",
        src.to_str().unwrap(),
    ]);

    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);

    // Pick one chunk id from the archive and delete its .cnk.
    let bytes = fs::read(&cfdir).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).unwrap();
    let chunk_id = arch
        .all_chunk_ids()
        .next()
        .expect("archive should reference at least one chunk");
    let hex_id = chunk_id.to_string();
    assert_eq!(hex_id.len(), 64);
    let cnk = store
        .join("chunks")
        .join(&hex_id[..2])
        .join(format!("{}.cnk", &hex_id[2..]));
    assert!(cnk.is_file(), "expected {}", cnk.display());
    fs::remove_file(&cnk).unwrap();

    let out = run_fail(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains(&hex_id),
        "error must include chunk id {hex_id}; stderr={err}"
    );

    // extract should also fail with the chunk id.
    let out_dir = dir.path().join("out");
    let ext = run_fail(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out_dir.to_str().unwrap(),
    ]);
    let ext_err = String::from_utf8_lossy(&ext.stderr);
    assert!(
        ext_err.contains(&hex_id),
        "extract error must include chunk id {hex_id}; stderr={ext_err}"
    );
}

#[test]
fn verify_cfidx_still_works_alongside_cfdir_dispatch() {
    // Regression: magic dispatch must not break single-blob verify.
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");
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
}

// --- Phase 5 M5: push / doctor / gc accept .cfdir ---

#[test]
fn doctor_and_gc_help_mention_cfdir() {
    for cmd in ["doctor", "gc", "push"] {
        let out = run_ok(&[cmd, "--help"]);
        let s = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
        assert!(
            s.contains("cfdir"),
            "{cmd} --help should mention .cfdir:\n{s}"
        );
        assert!(
            s.contains("cfidx"),
            "{cmd} --help should mention .cfidx:\n{s}"
        );
    }
}

#[test]
fn push_cfdir_to_mock_then_verify_source_succeeds() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"hello-tree\n").unwrap();
    fs::copy(
        fixtures_dir().join("hello.txt"),
        src.join("sub").join("b.txt"),
    )
    .unwrap();
    fs::copy(src.join("a.txt"), src.join("a-copy.txt")).unwrap();
    let cfdir = dir.path().join("release.cfdir");

    run_ok(&[
        "archive",
        "--store",
        local.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    let out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("uploaded="), "stderr={err}");
    assert!(
        err.contains("failed=0"),
        "push .cfdir should report failed=0; stderr={err}"
    );
    assert!(
        put_count.load(Ordering::SeqCst) >= 1,
        "expected at least one PUT for .cfdir push, got {}",
        put_count.load(Ordering::SeqCst)
    );
    assert!(
        count_cnk(&mirror.join("chunks")) >= 1,
        "mirror should contain uploaded .cnk files"
    );
    // Listing itself must not appear under the mirror.
    assert!(
        !mirror.join("release.cfdir").exists(),
        "push must not upload the .cfdir listing"
    );

    run_ok(&["verify", "--source", &base, cfdir.to_str().unwrap()]);
}

#[test]
fn doctor_cfdir_complete_and_missing() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"doctor-cfdir\n").unwrap();
    let cfdir = dir.path().join("tree.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("doctor: ok"), "stderr={err}");

    let bytes = fs::read(&cfdir).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).unwrap();
    let chunk_id = arch
        .all_chunk_ids()
        .next()
        .expect("archive should reference at least one chunk");
    let hex_id = chunk_id.to_string();
    let cnk = store
        .join("chunks")
        .join(&hex_id[..2])
        .join(format!("{}.cnk", &hex_id[2..]));
    fs::remove_file(&cnk).unwrap();

    let out = run_fail(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&hex_id),
        "missing id should appear on stdout; stdout={stdout}"
    );
}

#[test]
fn gc_cfdir_keeps_referenced_deletes_orphan() {
    let dir = tempdir().unwrap();
    let store_path = dir.path().join("store");
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"gc-cfdir\n").unwrap();
    let cfdir = dir.path().join("tree.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store_path.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let before = count_cnk(&store_path.join("chunks"));
    assert!(before >= 1, "store should have chunks");

    // Plant an orphan loose chunk.
    let orphan_plain = b"orphan-loose-chunk-for-gc-cfdir";
    {
        use chunkforge_store::Store;
        let s = Store::open(&store_path).unwrap();
        s.put(orphan_plain).unwrap();
    }
    let with_orphan = count_cnk(&store_path.join("chunks"));
    assert_eq!(with_orphan, before + 1);

    let out = run_ok(&[
        "gc",
        "--store",
        store_path.to_str().unwrap(),
        "--apply",
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("deleted") || err.contains("gc:"),
        "stderr={err}"
    );
    let after = count_cnk(&store_path.join("chunks"));
    assert_eq!(after, before, "orphan deleted; referenced retained");

    run_ok(&[
        "verify",
        "--store",
        store_path.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);
}

#[test]
fn push_mixed_cfidx_and_cfdir_union() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("tree-only.txt"), b"tree-unique-payload\n").unwrap();
    let cfdir = dir.path().join("tree.cfdir");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "archive",
        "--store",
        local.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    let out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        cfdir.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("failed=0"), "stderr={err}");
    assert!(
        put_count.load(Ordering::SeqCst) >= 1,
        "mixed push should PUT; got {}",
        put_count.load(Ordering::SeqCst)
    );

    run_ok(&["verify", "--source", &base, cfdir.to_str().unwrap()]);
    run_ok(&["verify", "--source", &base, idx.to_str().unwrap()]);
}

// --- Phase 5 M6: push --verify + archive --dry-run ---

/// PUT returns 200 but does not persist; HEAD/GET always 404 — push "succeeds", verify fails.
fn spawn_put_blackhole_server(put_count: Arc<AtomicUsize>) -> (String, thread::JoinHandle<()>) {
    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let handle = thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let method = request.method().clone();
            match method {
                Method::Head | Method::Get => {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
                Method::Put | Method::Post => {
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                    put_count.fetch_add(1, Ordering::SeqCst);
                    let _ = request.respond(
                        Response::empty(StatusCode(200))
                            .with_header(Header::from_bytes(&b"Content-Length"[..], "0").unwrap()),
                    );
                }
                _ => {
                    let _ = request.respond(Response::empty(StatusCode(405)));
                }
            }
        }
    });
    thread::sleep(Duration::from_millis(20));
    (base, handle)
}

#[test]
fn push_verify_cfdir_against_mock_succeeds() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"push-verify-tree\n").unwrap();
    fs::copy(
        fixtures_dir().join("hello.txt"),
        src.join("sub").join("b.txt"),
    )
    .unwrap();
    let cfdir = dir.path().join("release.cfdir");

    run_ok(&[
        "archive",
        "--store",
        local.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    let out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        "--verify",
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("failed=0"), "stderr={err}");
    assert!(
        err.contains("push: verify ok") || err.contains("verify: ok"),
        "expected post-push verify success; stderr={err}"
    );
    assert!(
        put_count.load(Ordering::SeqCst) >= 1,
        "expected PUT(s); got {}",
        put_count.load(Ordering::SeqCst)
    );
}

#[test]
fn push_verify_cfidx_against_mock_succeeds() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    let out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        "--verify",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("failed=0"), "stderr={err}");
    assert!(
        err.contains("push: verify ok") || err.contains("verify: ok"),
        "expected post-push verify success; stderr={err}"
    );
}

#[test]
fn push_verify_fails_when_remote_missing_chunk() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_blackhole_server(Arc::clone(&put_count));

    let out = run_fail(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        "--verify",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        put_count.load(Ordering::SeqCst) >= 1,
        "push should have issued PUT before verify"
    );
    assert!(
        err.contains("verify")
            && (err.contains("missing") || err.contains("fail") || err.contains("chunk")),
        "expected verify failure about missing chunk; stderr={err}"
    );
}

#[test]
fn push_verify_skipped_on_dry_run() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    let out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        "--dry-run",
        "--verify",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("--verify skipped") || err.contains("verify skipped"),
        "dry-run must skip verify; stderr={err}"
    );
    assert_eq!(put_count.load(Ordering::SeqCst), 0, "dry-run must not PUT");
    assert!(
        !err.contains("push: verify ok"),
        "must not claim verify ok; stderr={err}"
    );
}

#[test]
fn archive_dry_run_prints_stats_without_writing() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"dry-run-a\n").unwrap();
    fs::write(src.join("sub").join("b.txt"), b"dry-run-b\n").unwrap();
    // Identical twin → in-run would_reuse after first would_write of same content.
    fs::write(src.join("a-copy.txt"), b"dry-run-a\n").unwrap();
    let out_cfdir = dir.path().join("out.cfdir");

    let out = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out_cfdir.to_str().unwrap(),
        "--dry-run",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("dry-run") && err.contains("would_write="),
        "stderr={err}"
    );
    assert!(
        err.contains("no store") || err.contains("no store/.cfdir"),
        "stderr={err}"
    );
    // Phase6-M4: without --seed, dry-run must keep the 0.5.0 stats shape
    // (no would_seed_reuse / would_rechunk / seed_* noise).
    assert!(
        !err.contains("would_seed_reuse=")
            && !err.contains("would_rechunk=")
            && !err.contains("seed_reused_files=")
            && !err.contains("rechunked_files="),
        "no-seed dry-run must omit seed counters; stderr={err}"
    );
    assert!(!out_cfdir.exists(), "dry-run must not write .cfdir");
    assert!(
        !store.join("meta.toml").exists(),
        "dry-run must not create store when missing"
    );
}

#[test]
fn archive_help_lists_dry_run() {
    let help = run_ok(&["archive", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--dry-run"),
        "archive --help should list --dry-run:\n{s}"
    );
}

// --- Phase 6 M2/M3: archive --seed (+ dry-run×seed stats) ---

#[test]
fn archive_help_lists_seed() {
    let help = run_ok(&["archive", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--seed"),
        "archive --help should list --seed:\n{s}"
    );
    let lower = s.to_lowercase();
    assert!(
        lower.contains("content") && (lower.contains("fingerprint") || lower.contains("blake3")),
        "archive --help --seed should mention content fingerprint semantics:\n{s}"
    );
    assert!(
        lower.contains("chunk table") || lower.contains("chunk tables") || lower.contains("reuse"),
        "archive --help --seed should mention reuse of chunk tables:\n{s}"
    );
    assert!(
        s.contains("--jobs"),
        "archive --help should list --jobs:\n{s}"
    );
}

#[test]
fn archive_jobs_four_matches_serial_listing() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"jobs-a\n").unwrap();
    fs::write(src.join("sub/b.txt"), b"jobs-b\n").unwrap();
    fs::write(src.join("c.txt"), b"jobs-c\n").unwrap();
    fs::write(src.join("d.txt"), b"jobs-d\n").unwrap();

    let store1 = dir.path().join("store1");
    let store4 = dir.path().join("store4");
    let out1 = dir.path().join("j1.cfdir");
    let out4 = dir.path().join("j4.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store1.to_str().unwrap(),
        "-o",
        out1.to_str().unwrap(),
        "--jobs",
        "1",
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "archive",
        "--store",
        store4.to_str().unwrap(),
        "-o",
        out4.to_str().unwrap(),
        "--jobs",
        "4",
        src.to_str().unwrap(),
    ]);

    let b1 = fs::read(&out1).unwrap();
    let b4 = fs::read(&out4).unwrap();
    assert_eq!(
        b1, b4,
        "archive --jobs 1 and --jobs 4 must produce identical .cfdir bytes"
    );
    run_ok(&[
        "verify",
        "--store",
        store4.to_str().unwrap(),
        out4.to_str().unwrap(),
    ]);
}

#[test]
fn archive_seed_same_tree_reuses_all_files_verify_green() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    let sub = src.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(src.join("a.txt"), b"hello-seed-v1\n").unwrap();
    fs::write(sub.join("b.txt"), b"shared-payload\n").unwrap();
    fs::write(src.join("a-copy.txt"), b"hello-seed-v1\n").unwrap();

    let store = dir.path().join("store");
    let prior = dir.path().join("v1.cfdir");
    let seeded = dir.path().join("v1b.cfdir");

    let first = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        prior.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err1 = String::from_utf8_lossy(&first.stderr);
    assert!(err1.contains("archive: wrote"), "stderr={err1}");
    // No --seed → no seed counters required.
    assert!(
        !err1.contains("seed_reused_files="),
        "no-seed archive should omit seed counters; stderr={err1}"
    );

    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        prior.to_str().unwrap(),
    ]);

    let store_h = chunkforge_store::Store::open(&store).expect("open store");
    let chunk_count_before = store_h.list_chunk_ids().expect("list").len();

    let second = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        seeded.to_str().unwrap(),
        "--seed",
        prior.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err2 = String::from_utf8_lossy(&second.stderr);
    assert!(
        err2.contains("seed_reused_files=3"),
        "same-tree seed should reuse all 3 files; stderr={err2}"
    );
    assert!(
        err2.contains("rechunked_files=0"),
        "same-tree seed should rechunk 0; stderr={err2}"
    );
    assert!(
        err2.contains("new=0"),
        "same-tree seed should write no new chunks; stderr={err2}"
    );

    let chunk_count_after = store_h.list_chunk_ids().expect("list again").len();
    assert_eq!(
        chunk_count_before, chunk_count_after,
        "store must not grow on identical --seed archive"
    );

    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        seeded.to_str().unwrap(),
    ]);
}

#[test]
fn archive_seed_changed_file_only_rechunked() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    let sub = src.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(src.join("a.txt"), b"hello-seed-v1\n").unwrap();
    let hello = fs::read(fixtures_dir().join("hello.txt")).expect("fixtures/hello.txt");
    fs::write(sub.join("b.txt"), &hello).unwrap();
    fs::write(src.join("c.txt"), b"unchanged-c\n").unwrap();

    let store = dir.path().join("store");
    let v1 = dir.path().join("v1.cfdir");
    let v2 = dir.path().join("v2.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    // Change only a.txt
    fs::write(src.join("a.txt"), b"hello-seed-v2\n").unwrap();

    let store_h = chunkforge_store::Store::open(&store).expect("open store");
    let before = store_h.list_chunk_ids().expect("list").len();

    let out = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v2.to_str().unwrap(),
        "--seed",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("seed_reused_files=2"),
        "two unchanged files should reuse; stderr={err}"
    );
    assert!(
        err.contains("rechunked_files=1"),
        "only a.txt should rechunk; stderr={err}"
    );

    let after = store_h.list_chunk_ids().expect("list").len();
    assert!(
        after >= before,
        "store may gain chunks for changed file ({before} → {after})"
    );
    // Changed file is small; expect few new chunks (not a flood).
    assert!(
        after - before <= 4,
        "only one small file changed; unexpected chunk growth {before} → {after}"
    );

    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        v2.to_str().unwrap(),
    ]);
}

#[test]
fn archive_seed_missing_chunk_forces_rechunk_and_warns() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    // Multi-chunk file so deleting one .cnk is meaningful.
    let mut data = Vec::with_capacity(24 * 1024);
    for i in 0..(24 * 1024) {
        data.push(((i * 17 + 3) % 251) as u8);
    }
    fs::write(src.join("big.bin"), &data).unwrap();
    fs::write(src.join("sub/small.txt"), b"keep-me\n").unwrap();

    let store = dir.path().join("store");
    let v1 = dir.path().join("v1.cfdir");
    let v2 = dir.path().join("v2.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v1.to_str().unwrap(),
        "--chunk-size",
        "2048:4096:8192",
        src.to_str().unwrap(),
    ]);

    // Delete one chunk belonging to big.bin.
    let bytes = fs::read(&v1).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).unwrap();
    let big = arch
        .entries
        .iter()
        .find(|e| e.path == "big.bin")
        .expect("big.bin entry");
    let chunk_id = match &big.kind {
        chunkforge_index::DirEntryKind::File { chunks, .. } => {
            chunks.first().expect("big.bin has chunks").chunk_id
        }
        _ => panic!("big.bin should be File"),
    };
    let hex_id = chunk_id.to_string();
    let cnk = store
        .join("chunks")
        .join(&hex_id[..2])
        .join(format!("{}.cnk", &hex_id[2..]));
    assert!(cnk.is_file(), "expected {}", cnk.display());
    fs::remove_file(&cnk).unwrap();

    let out = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v2.to_str().unwrap(),
        "--seed",
        v1.to_str().unwrap(),
        "--chunk-size",
        "2048:4096:8192",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("missing") && err.contains("rechunk"),
        "expected missing-chunk warning; stderr={err}"
    );
    assert!(
        err.contains("seed_missing_chunks=1") || err.contains("rechunked_files=1"),
        "big.bin should rechunk; stderr={err}"
    );
    assert!(
        err.contains("seed_reused_files=1"),
        "small.txt should still reuse; stderr={err}"
    );

    // New listing must verify green (not a broken silent reuse).
    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        v2.to_str().unwrap(),
    ]);
}

#[test]
fn archive_seed_rejects_cfidx_and_bad_magic() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let store = dir.path().join("store");
    let out = dir.path().join("out.cfdir");

    // Make a .cfidx to misuse as seed.
    let idx = dir.path().join("single.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let fail_idx = run_fail(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--seed",
        idx.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail_idx.stderr);
    assert!(
        err.contains("cfdir") || err.contains(".cfidx") || err.contains("seed"),
        "should reject .cfidx seed; stderr={err}"
    );

    let junk = dir.path().join("junk.cfdir");
    fs::write(&junk, b"NOTACFDIR").unwrap();
    let fail_magic = run_fail(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--seed",
        junk.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err2 = String::from_utf8_lossy(&fail_magic.stderr);
    assert!(
        !fail_magic.status.success(),
        "bad magic must fail; stderr={err2}"
    );
}

#[test]
fn archive_dry_run_with_seed_reports_reuse_without_writing() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"dry-seed-a\n").unwrap();
    fs::write(src.join("b.txt"), b"dry-seed-b\n").unwrap();

    let store = dir.path().join("store");
    let prior = dir.path().join("prior.cfdir");
    let out = dir.path().join("would.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        prior.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let before = count_cnk(&store.join("chunks"));

    let dry = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--seed",
        prior.to_str().unwrap(),
        "--dry-run",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&dry.stderr);
    assert!(err.contains("dry-run"), "stderr={err}");
    assert!(
        err.contains("would_seed_reuse=2") && err.contains("would_rechunk=0"),
        "dry-run×seed should preview would_seed_reuse / would_rechunk; stderr={err}"
    );
    // Dry-run vocabulary must not reuse the write-path counter names.
    assert!(
        !err.contains("seed_reused_files=") && !err.contains("rechunked_files="),
        "dry-run×seed should use would_* counters, not write-path names; stderr={err}"
    );
    assert!(!out.exists(), "dry-run must not write .cfdir");
    assert_eq!(
        before,
        count_cnk(&store.join("chunks")),
        "dry-run must not add chunks"
    );
    // Store directory itself must not be created if absent; here it already
    // exists from the prior archive — chunk count must stay unchanged.
}

#[test]
fn archive_dry_run_with_seed_changed_file_would_rechunk() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"dry-seed-a-v1\n").unwrap();
    fs::write(src.join("b.txt"), b"dry-seed-b\n").unwrap();

    let store = dir.path().join("store");
    let prior = dir.path().join("prior.cfdir");
    let out = dir.path().join("would.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        prior.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let before = count_cnk(&store.join("chunks"));
    let prior_mtime = fs::metadata(&prior).unwrap().modified().unwrap();

    fs::write(src.join("a.txt"), b"dry-seed-a-v2\n").unwrap();

    let dry = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--seed",
        prior.to_str().unwrap(),
        "--dry-run",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&dry.stderr);
    assert!(
        err.contains("would_seed_reuse=1") && err.contains("would_rechunk=1"),
        "dry-run×seed should preview one reuse + one rechunk; stderr={err}"
    );
    assert!(!out.exists(), "dry-run must not write .cfdir");
    assert_eq!(
        before,
        count_cnk(&store.join("chunks")),
        "dry-run must not add chunks"
    );
    let after_mtime = fs::metadata(&prior).unwrap().modified().unwrap();
    assert_eq!(
        prior_mtime, after_mtime,
        "dry-run must not touch the prior .cfdir"
    );
}

// --- Phase 6 M4: no-seed / legacy path regression (0.5.0 compat) ---

/// Without `--seed`, archive (+ dry-run) must keep the 0.5.0 contract:
/// legacy stats shape, store+.cfdir writes (or neither on dry-run), and
/// `.cfidx` make / `push --verify` on both listing kinds stay green.
#[test]
fn phase6_m4_no_seed_legacy_path_regression() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"m4-legacy-a\n").unwrap();
    fs::write(src.join("sub").join("b.txt"), b"m4-legacy-b\n").unwrap();
    fs::write(src.join("a-copy.txt"), b"m4-legacy-a\n").unwrap();

    let store = dir.path().join("store");
    let cfdir = dir.path().join("release.cfdir");
    let dry_cfdir = dir.path().join("dry.cfdir");
    let dry_store = dir.path().join("dry-store");
    let cfidx = dir.path().join("hello.cfidx");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();

    // --- dry-run without --seed: no writes, legacy would_* only ---
    let dry = run_ok(&[
        "archive",
        "--store",
        dry_store.to_str().unwrap(),
        "-o",
        dry_cfdir.to_str().unwrap(),
        "--dry-run",
        src.to_str().unwrap(),
    ]);
    let dry_err = String::from_utf8_lossy(&dry.stderr);
    assert!(
        dry_err.contains("dry-run")
            && dry_err.contains("would_write=")
            && dry_err.contains("would_reuse="),
        "no-seed dry-run should use would_write/would_reuse; stderr={dry_err}"
    );
    assert!(
        !dry_err.contains("would_seed_reuse=")
            && !dry_err.contains("would_rechunk=")
            && !dry_err.contains("seed_reused_files="),
        "no-seed dry-run must omit seed counters; stderr={dry_err}"
    );
    assert!(!dry_cfdir.exists(), "dry-run must not write .cfdir");
    assert!(
        !dry_store.join("meta.toml").exists(),
        "dry-run must not create store"
    );

    // --- write path without --seed: store + .cfdir, legacy new=/reused= ---
    let wrote = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let wrote_err = String::from_utf8_lossy(&wrote.stderr);
    assert!(
        wrote_err.contains("archive: wrote")
            && wrote_err.contains("new=")
            && wrote_err.contains("reused="),
        "no-seed archive should print legacy stats; stderr={wrote_err}"
    );
    assert!(
        !wrote_err.contains("seed_reused_files=") && !wrote_err.contains("rechunked_files="),
        "no-seed archive must omit seed counters; stderr={wrote_err}"
    );
    assert!(cfdir.is_file(), "archive must write .cfdir");
    assert!(
        store.join("meta.toml").is_file(),
        "archive must create store"
    );
    assert!(
        count_cnk(&store.join("chunks")) >= 1,
        "archive must write chunks"
    );

    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);

    // --- .cfidx make + verify still works alongside .cfdir ---
    let hello = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfidx.to_str().unwrap(),
        hello.to_str().unwrap(),
    ]);
    assert!(cfidx.is_file(), "make must write .cfidx");
    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        cfidx.to_str().unwrap(),
    ]);

    // --- push --verify on .cfdir and .cfidx ---
    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    let push_dir = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base,
        "--verify",
        cfdir.to_str().unwrap(),
    ]);
    let push_dir_err = String::from_utf8_lossy(&push_dir.stderr);
    assert!(push_dir_err.contains("failed=0"), "stderr={push_dir_err}");
    assert!(
        push_dir_err.contains("push: verify ok") || push_dir_err.contains("verify: ok"),
        "push --verify .cfdir should succeed; stderr={push_dir_err}"
    );

    let push_idx = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base,
        "--verify",
        cfidx.to_str().unwrap(),
    ]);
    let push_idx_err = String::from_utf8_lossy(&push_idx.stderr);
    assert!(push_idx_err.contains("failed=0"), "stderr={push_idx_err}");
    assert!(
        push_idx_err.contains("push: verify ok") || push_idx_err.contains("verify: ok"),
        "push --verify .cfidx should succeed; stderr={push_idx_err}"
    );
    assert!(
        put_count.load(Ordering::SeqCst) >= 1,
        "expected at least one PUT across push --verify; got {}",
        put_count.load(Ordering::SeqCst)
    );
}

// --- Phase 6 M5: chunkforge pull ---

#[test]
fn pull_help_lists_store_source_dry_run_templates() {
    let help = run_ok(&["--help"]);
    let top = String::from_utf8_lossy(&help.stdout);
    assert!(
        top.contains("pull"),
        "top-level help should list pull:\n{top}"
    );

    let p = run_ok(&["pull", "--help"]);
    let s = String::from_utf8_lossy(&p.stdout);
    assert!(s.contains("--store"), "{s}");
    assert!(s.contains("--source"), "{s}");
    assert!(s.contains("--dry-run"), "{s}");
    assert!(s.contains("--verify"), "{s}");
    assert!(s.contains("--jobs"), "{s}");
    assert!(s.contains("--url-template"), "{s}");
    assert!(s.contains("--prefix"), "{s}");
    assert!(s.contains("--header"), "{s}");
    assert!(
        s.to_ascii_lowercase().contains("cfidx"),
        "help should mention .cfidx:\n{s}"
    );
    assert!(
        s.to_ascii_lowercase().contains("cfdir"),
        "help should mention .cfdir:\n{s}"
    );
}

#[test]
fn pull_from_mock_after_push_then_verify_store_green() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    let newstore = dir.path().join("newstore");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        idx.to_str().unwrap(),
    ]);
    assert!(
        put_count.load(Ordering::SeqCst) >= 1,
        "push should upload before pull"
    );

    // Empty destination CAS + listing only.
    fs::create_dir_all(&newstore).unwrap();
    let out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        &base,
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("fetched="), "stderr={err}");
    assert!(
        err.contains("failed=0"),
        "pull should report failed=0; stderr={err}"
    );
    assert!(
        !err.contains("fetched=0"),
        "empty store should fetch; stderr={err}"
    );
    assert!(
        count_cnk(&newstore.join("chunks")) >= 1,
        "newstore should contain pulled .cnk files"
    );

    run_ok(&[
        "verify",
        "--store",
        newstore.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
}

#[test]
fn pull_dry_run_does_not_write_store() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    let newstore = dir.path().join("newstore");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));
    run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        idx.to_str().unwrap(),
    ]);

    // Intentionally do not create newstore — dry-run must not create meta/chunks.
    let out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        &base,
        "--dry-run",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("dry_run=true") || err.contains("fetched="),
        "stderr={err}"
    );
    assert!(
        !err.contains("fetched=0"),
        "dry-run against empty store should count fetches; stderr={err}"
    );
    assert!(
        !newstore.join("meta.toml").is_file(),
        "dry-run must not create store meta.toml"
    );
    assert_eq!(
        count_cnk(&newstore.join("chunks")),
        0,
        "dry-run must not write .cnk files"
    );
}

#[test]
fn pull_skips_already_present_chunks() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    let newstore = dir.path().join("newstore");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));
    run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        idx.to_str().unwrap(),
    ]);

    run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        &base,
        idx.to_str().unwrap(),
    ]);
    let first_cnk = count_cnk(&newstore.join("chunks"));
    assert!(first_cnk >= 1);

    let out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        &base,
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("fetched=0"),
        "second pull should fetch nothing; stderr={err}"
    );
    assert!(
        err.contains("skipped=") && !err.contains("skipped=0"),
        "second pull should skip present chunks; stderr={err}"
    );
    assert_eq!(
        count_cnk(&newstore.join("chunks")),
        first_cnk,
        "idempotent pull must not add chunks"
    );
}

#[test]
fn pull_from_local_source_path() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("failed=0"), "stderr={err}");
    assert!(
        !err.contains("fetched=0"),
        "should fetch from local source; stderr={err}"
    );

    run_ok(&[
        "verify",
        "--store",
        newstore.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
}

#[test]
fn pull_cfdir_from_mock_then_verify() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    let newstore = dir.path().join("newstore");
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::create_dir_all(&mirror).unwrap();
    fs::write(src.join("a.txt"), b"pull-cfdir-a\n").unwrap();
    fs::write(src.join("sub/b.txt"), b"pull-cfdir-b\n").unwrap();
    let cfdir = dir.path().join("tree.cfdir");

    run_ok(&[
        "archive",
        "--store",
        local.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));
    run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        cfdir.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        &base,
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("failed=0"), "stderr={err}");

    run_ok(&[
        "verify",
        "--store",
        newstore.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);
}

// --- Phase 18 M1: pull --verify (symmetric to push --verify) ---

#[test]
fn pull_verify_cfidx_against_local_store_succeeds() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--verify",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("failed=0"), "stderr={err}");
    assert!(
        err.contains("pull: verify ok") || err.contains("verify: ok"),
        "expected post-pull verify success; stderr={err}"
    );
    assert!(
        err.contains("pull: verifying"),
        "expected verifying notice; stderr={err}"
    );
}

#[test]
fn pull_verify_cfdir_against_local_store_succeeds() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"pull-verify-tree\n").unwrap();
    fs::copy(
        fixtures_dir().join("hello.txt"),
        src.join("sub").join("b.txt"),
    )
    .unwrap();
    let cfdir = dir.path().join("release.cfdir");

    run_ok(&[
        "archive",
        "--store",
        local.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--verify",
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("failed=0"), "stderr={err}");
    assert!(
        err.contains("pull: verify ok") || err.contains("verify: ok"),
        "expected post-pull verify success; stderr={err}"
    );
}

#[test]
fn pull_verify_fails_when_store_chunk_corrupt() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // Fill newstore first (no --verify).
    run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);

    // Corrupt a present .cnk in place: has() stays true → pull skips fetch;
    // post-pull --verify get() then fails hash check → non-zero.
    let (cnk_path, _id) = first_cnk_id(&newstore.join("chunks"));
    let len = fs::metadata(&cnk_path).unwrap().len() as usize;
    fs::write(&cnk_path, vec![0u8; len.max(1)]).unwrap();

    let out = run_fail(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--verify",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("verify")
            && (err.contains("mismatch")
                || err.contains("corrupt")
                || err.contains("fail")
                || err.contains("unreadable")
                || err.contains("hash")),
        "expected verify failure about corrupt chunk; stderr={err}"
    );
}

#[test]
fn pull_verify_fails_when_path_filter_left_listing_incomplete() {
    // path filter shrinks the fetch set, but --verify still runs full listing
    // (same as push --verify over index_paths) → missing chunks → non-zero.
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("keep")).unwrap();
    fs::create_dir_all(src.join("skip")).unwrap();
    fs::write(src.join("keep").join("a.txt"), b"keep-me-unique-aaa\n").unwrap();
    fs::write(src.join("skip").join("b.txt"), b"skip-me-unique-bbb\n").unwrap();
    let cfdir = dir.path().join("tree.cfdir");

    run_ok(&[
        "archive",
        "--store",
        local.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = run_fail(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--path",
        "keep/",
        "--verify",
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("verify")
            && (err.contains("missing") || err.contains("fail") || err.contains("chunk")),
        "expected verify failure for unfetched path; stderr={err}"
    );
}

#[test]
fn pull_verify_skipped_on_dry_run() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--dry-run",
        "--verify",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("--verify skipped") || err.contains("verify skipped"),
        "dry-run must skip verify; stderr={err}"
    );
    assert!(
        !err.contains("pull: verify ok"),
        "dry-run must not claim verify ok; stderr={err}"
    );
    assert!(
        !newstore.join("meta.toml").is_file(),
        "dry-run must not create store"
    );
}

#[test]
fn pull_without_verify_flag_is_quiet() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !err.to_ascii_lowercase().contains("verifying"),
        "no --verify must stay quiet; stderr={err}"
    );
    assert!(
        !err.contains("pull: verify ok"),
        "no --verify must not emit verify ok; stderr={err}"
    );
}

#[test]
fn pull_missing_source_chunk_fails() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let empty_src = dir.path().join("empty_src");
    let newstore = dir.path().join("newstore");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    // Empty-but-valid store as source → get fails for referenced ids.
    run_ok(&[
        "make",
        "--store",
        empty_src.to_str().unwrap(),
        "-o",
        dir.path().join("other.cfidx").to_str().unwrap(),
        // tiny distinct content so store exists with different chunks
        input.to_str().unwrap(),
    ]);
    delete_cnk_files(&empty_src.join("chunks"));

    let out = run_fail(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        empty_src.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("fail") || err.contains("missing") || err.contains("not found"),
        "stderr={err}"
    );
}

#[test]
fn pull_rejects_http_templates_on_local_source() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_fail(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--url-template",
        "{base}/{path}",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("url-template")
            || err.contains("prefix")
            || err.contains("header")
            || err.contains("http"),
        "stderr={err}"
    );
}

// --- Phase 7 M2: chunkforge diff ---

fn parse_diff_summary(stdout: &str) -> &str {
    stdout
        .lines()
        .rev()
        .find(|l| l.starts_with("diff: "))
        .expect("missing diff: summary line")
}

#[test]
fn diff_help_lists_max_paths_and_stdout_summary() {
    let out = run_ok(&["diff", "--help"]);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("--max-paths"),
        "diff --help should list --max-paths:\n{s}"
    );
    assert!(
        s.contains("stdout") || s.contains("summary"),
        "diff --help should document summary on stdout:\n{s}"
    );
    assert!(
        s.contains("cfdir") || s.contains(".cfdir"),
        "diff --help should mention .cfdir:\n{s}"
    );
    assert!(s.contains("--tree"), "diff --help should list --tree:\n{s}");
    assert!(
        s.contains("--format"),
        "diff --help should list --format:\n{s}"
    );
    assert!(
        s.contains("json") || s.contains("text"),
        "diff --help should mention text/json formats:\n{s}"
    );
}

#[test]
fn diff_identical_cfdirs_exit_zero_all_zeros() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"hello-diff-identical\n").unwrap();
    fs::write(src.join("sub").join("b.txt"), b"shared\n").unwrap();

    let store = dir.path().join("store");
    let left = dir.path().join("left.cfdir");
    let right = dir.path().join("right.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        left.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    // Second archive of the same tree → identical listing content for File paths.
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        right.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = run_ok(&["diff", left.to_str().unwrap(), right.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let summary = parse_diff_summary(&stdout);
    assert_eq!(
        summary,
        "diff: added=0 removed=0 changed=0 meta_changed=0 chunks_shared=2 chunks_only_left=0 chunks_only_right=0",
        "stdout={stdout}"
    );
    assert!(
        !stdout.contains("added:\n")
            && !stdout.contains("removed:\n")
            && !stdout.contains("changed:\n")
            && !stdout.contains("meta_changed:\n"),
        "identical diff should omit empty path categories; stdout={stdout}"
    );
}

#[test]
fn diff_changed_file_exit_nonzero() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"hello-diff-v1\n").unwrap();
    let hello = fs::read(fixtures_dir().join("hello.txt")).expect("fixtures/hello.txt");
    fs::write(src.join("sub").join("b.txt"), &hello).unwrap();

    let store = dir.path().join("store");
    let v1 = dir.path().join("v1.cfdir");
    let v2 = dir.path().join("v2.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    fs::write(src.join("a.txt"), b"hello-diff-v2\n").unwrap();
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v2.to_str().unwrap(),
        "--seed",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = run_fail(&["diff", v1.to_str().unwrap(), v2.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "diff with changes should exit 1; status={:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let summary = parse_diff_summary(&stdout);
    assert!(
        summary.contains("changed=1"),
        "expected changed=1 in summary; got {summary}; stdout={stdout}"
    );
    assert!(
        summary.contains("added=0") && summary.contains("removed=0"),
        "only content change expected; summary={summary}"
    );
    assert!(
        stdout.contains("changed:") && stdout.contains("a.txt"),
        "should list changed path a.txt; stdout={stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("error:"),
        "differences must not print error: prefix; stderr={stderr}"
    );
}

#[test]
fn diff_rejects_cfidx() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("single.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // Need a real cfdir for the other side so we exercise cfidx rejection.
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let cfdir = dir.path().join("tree.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let fail_left = run_fail(&["diff", idx.to_str().unwrap(), cfdir.to_str().unwrap()]);
    let err = String::from_utf8_lossy(&fail_left.stderr);
    assert!(
        err.contains("cfidx") || err.contains(".cfidx") || err.contains("cfdir"),
        "should reject .cfidx; stderr={err}"
    );

    let fail_right = run_fail(&["diff", cfdir.to_str().unwrap(), idx.to_str().unwrap()]);
    let err2 = String::from_utf8_lossy(&fail_right.stderr);
    assert!(
        err2.contains("cfidx") || err2.contains(".cfidx") || err2.contains("cfdir"),
        "should reject .cfidx on right; stderr={err2}"
    );
}

#[test]
fn diff_max_paths_truncates_listing() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    for i in 0..5 {
        fs::write(src.join(format!("f{i}.txt")), format!("content-{i}\n")).unwrap();
    }

    let store = dir.path().join("store");
    let v1 = dir.path().join("v1.cfdir");
    let v2 = dir.path().join("v2.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    // Add two new files on the right.
    fs::write(src.join("new0.txt"), b"new0\n").unwrap();
    fs::write(src.join("new1.txt"), b"new1\n").unwrap();
    fs::write(src.join("new2.txt"), b"new2\n").unwrap();
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v2.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = run_fail(&[
        "diff",
        "--max-paths",
        "1",
        v1.to_str().unwrap(),
        v2.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("added:"),
        "should have added category; stdout={stdout}"
    );
    assert!(
        stdout.contains("... and 2 more"),
        "max-paths=1 with 3 added should truncate; stdout={stdout}"
    );
    let summary = parse_diff_summary(&stdout);
    assert!(
        summary.contains("added=3"),
        "summary counts must stay full; summary={summary}"
    );
}

// --- Phase 16 M6 / P1 O1: diff --path / --exclude / --exclude-from ---

#[test]
fn diff_path_exclude_narrows_before_compare() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("keep")).unwrap();
    fs::create_dir_all(src.join("skip")).unwrap();
    fs::write(src.join("keep").join("a.txt"), b"keep-a-v1\n").unwrap();
    fs::write(src.join("skip").join("b.txt"), b"skip-b-v1\n").unwrap();

    let store = dir.path().join("store");
    let v1 = dir.path().join("v1.cfdir");
    let v2 = dir.path().join("v2.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    // Change only under skip/ — full diff sees changed; --path keep/ hides it.
    fs::write(src.join("skip").join("b.txt"), b"skip-b-v2\n").unwrap();
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v2.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let full = run_fail(&["diff", v1.to_str().unwrap(), v2.to_str().unwrap()]);
    let full_stdout = String::from_utf8_lossy(&full.stdout);
    let full_summary = parse_diff_summary(&full_stdout);
    assert!(
        full_summary.contains("changed=1"),
        "unfiltered diff should see skip change; summary={full_summary}"
    );

    let scoped = run_ok(&[
        "diff",
        "--path",
        "keep",
        v1.to_str().unwrap(),
        v2.to_str().unwrap(),
    ]);
    let scoped_out = String::from_utf8_lossy(&scoped.stdout);
    let scoped_summary = parse_diff_summary(&scoped_out);
    assert_eq!(
        scoped_summary,
        "diff: added=0 removed=0 changed=0 meta_changed=0 chunks_shared=1 chunks_only_left=0 chunks_only_right=0",
        "stdout={scoped_out}"
    );

    let excl = run_ok(&[
        "diff",
        "--exclude",
        "skip/",
        v1.to_str().unwrap(),
        v2.to_str().unwrap(),
    ]);
    let excl_stdout = String::from_utf8_lossy(&excl.stdout);
    let excl_summary = parse_diff_summary(&excl_stdout);
    assert!(
        excl_summary.contains("changed=0")
            && excl_summary.contains("added=0")
            && excl_summary.contains("removed=0"),
        "--exclude skip/ should hide the change; summary={excl_summary}"
    );

    let from_file = dir.path().join("excludes.txt");
    fs::write(&from_file, "skip/\n").unwrap();
    let from = run_ok(&[
        "diff",
        "--exclude-from",
        from_file.to_str().unwrap(),
        v1.to_str().unwrap(),
        v2.to_str().unwrap(),
    ]);
    let from_stdout = String::from_utf8_lossy(&from.stdout);
    let from_summary = parse_diff_summary(&from_stdout);
    assert!(
        from_summary.contains("changed=0"),
        "--exclude-from should match --exclude; summary={from_summary}"
    );

    let help = run_ok(&["diff", "--help"]);
    let help_s = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_s.contains("--path")
            && help_s.contains("--exclude")
            && help_s.contains("--exclude-from"),
        "diff --help should list path filter flags:\n{help_s}"
    );
}

// --- Phase 7 M3: diff --tree ---

#[test]
fn diff_tree_matches_listing_exit_zero() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"hello-tree-match\n").unwrap();
    fs::write(src.join("sub").join("b.txt"), b"shared-tree\n").unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("listing.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    // Snapshot store + listing before --tree (must stay read-only).
    let store_before = snapshot_store_files(&store);
    let listing_mtime_before = fs::metadata(&listing).unwrap().modified().unwrap();
    let listing_bytes_before = fs::read(&listing).unwrap();
    let cfdirs_before = count_cfdirs(dir.path());

    let out = run_ok(&[
        "diff",
        "--tree",
        src.to_str().unwrap(),
        listing.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let summary = parse_diff_summary(&stdout);
    assert!(
        summary.contains("added=0")
            && summary.contains("removed=0")
            && summary.contains("changed=0")
            && summary.contains("meta_changed=0")
            && summary.contains("chunks_only_left=0")
            && summary.contains("chunks_only_right=0"),
        "identical tree↔listing should be clean; summary={summary}; stdout={stdout}"
    );
    assert!(
        summary.contains("chunks_shared=2"),
        "matching content should copy listing chunk tables; summary={summary}"
    );

    let store_after = snapshot_store_files(&store);
    assert_eq!(
        store_before, store_after,
        "diff --tree must not write store objects"
    );
    let listing_mtime_after = fs::metadata(&listing).unwrap().modified().unwrap();
    assert_eq!(
        listing_mtime_before, listing_mtime_after,
        "diff --tree must not touch the listing file mtime"
    );
    assert_eq!(
        listing_bytes_before,
        fs::read(&listing).unwrap(),
        "diff --tree must not rewrite the listing"
    );
    assert_eq!(
        cfdirs_before,
        count_cfdirs(dir.path()),
        "diff --tree must not write a new .cfdir"
    );
}

#[test]
fn diff_tree_changed_byte_exit_nonzero() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"hello-tree-v1\n").unwrap();
    let hello = fs::read(fixtures_dir().join("hello.txt")).expect("fixtures/hello.txt");
    fs::write(src.join("sub").join("b.txt"), &hello).unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("listing.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let store_before = snapshot_store_files(&store);
    let cfdirs_before = count_cfdirs(dir.path());

    // Change one byte in the source tree (do not re-archive).
    fs::write(src.join("a.txt"), b"hello-tree-v2\n").unwrap();

    let out = run_fail(&[
        "diff",
        "--tree",
        src.to_str().unwrap(),
        listing.to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "changed tree should exit 1; status={:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let summary = parse_diff_summary(&stdout);
    assert!(
        summary.contains("changed=1"),
        "expected changed=1; summary={summary}; stdout={stdout}"
    );
    assert!(
        summary.contains("added=0") && summary.contains("removed=0"),
        "only content change expected; summary={summary}"
    );
    assert!(
        stdout.contains("changed:") && stdout.contains("a.txt"),
        "should list changed path a.txt; stdout={stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("error:"),
        "differences must not print error: prefix; stderr={stderr}"
    );

    assert_eq!(
        store_before,
        snapshot_store_files(&store),
        "diff --tree must not write store after content change"
    );
    assert_eq!(
        cfdirs_before,
        count_cfdirs(dir.path()),
        "diff --tree must not write a new .cfdir after content change"
    );
}

#[test]
fn diff_tree_rejects_extra_listing_arg() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let store = dir.path().join("store");
    let listing = dir.path().join("listing.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let extra = dir.path().join("extra.cfdir");
    fs::copy(&listing, &extra).unwrap();

    let fail = run_fail(&[
        "diff",
        "--tree",
        src.to_str().unwrap(),
        listing.to_str().unwrap(),
        extra.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("--tree") || err.contains("extra") || err.contains("unexpected"),
        "should reject two listings with --tree; stderr={err}"
    );
}

#[test]
fn diff_without_tree_rejects_bare_directory() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let store = dir.path().join("store");
    let listing = dir.path().join("listing.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let fail = run_fail(&["diff", src.to_str().unwrap(), listing.to_str().unwrap()]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("--tree") || err.contains("directory"),
        "bare dir without --tree should error helpfully; stderr={err}"
    );
}

// --- Phase 8 M4: diff --format json ---

fn parse_diff_summary_counts(summary: &str) -> (usize, usize, usize, usize, usize, usize, usize) {
    let mut added = None;
    let mut removed = None;
    let mut changed = None;
    let mut meta_changed = None;
    let mut chunks_shared = None;
    let mut chunks_only_left = None;
    let mut chunks_only_right = None;
    let body = summary
        .strip_prefix("diff: ")
        .unwrap_or_else(|| panic!("expected diff: prefix; got {summary}"));
    for part in body.split_whitespace() {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let n: usize = v
            .parse()
            .unwrap_or_else(|_| panic!("bad count {part} in {summary}"));
        match k {
            "added" => added = Some(n),
            "removed" => removed = Some(n),
            "changed" => changed = Some(n),
            "meta_changed" => meta_changed = Some(n),
            "chunks_shared" => chunks_shared = Some(n),
            "chunks_only_left" => chunks_only_left = Some(n),
            "chunks_only_right" => chunks_only_right = Some(n),
            _ => {}
        }
    }
    (
        added.expect("added"),
        removed.expect("removed"),
        changed.expect("changed"),
        meta_changed.expect("meta_changed"),
        chunks_shared.expect("chunks_shared"),
        chunks_only_left.expect("chunks_only_left"),
        chunks_only_right.expect("chunks_only_right"),
    )
}

fn parse_diff_json_counts(json: &str) -> (usize, usize, usize, usize, usize, usize, usize) {
    let v: serde_json::Value = serde_json::from_str(json.trim())
        .unwrap_or_else(|e| panic!("invalid json: {e}; body={json}"));
    let arr_len = |key: &str| -> usize {
        v.get(key)
            .unwrap_or_else(|| panic!("missing {key} in {json}"))
            .as_array()
            .unwrap_or_else(|| panic!("{key} must be array in {json}"))
            .len()
    };
    let num = |key: &str| -> usize {
        v.get(key)
            .unwrap_or_else(|| panic!("missing {key} in {json}"))
            .as_u64()
            .unwrap_or_else(|| panic!("{key} must be number in {json}")) as usize
    };
    (
        arr_len("added"),
        arr_len("removed"),
        arr_len("changed"),
        arr_len("meta_changed"),
        num("chunks_shared"),
        num("chunks_only_left"),
        num("chunks_only_right"),
    )
}

#[test]
fn diff_format_json_counts_match_text() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"hello-format-v1\n").unwrap();
    fs::write(src.join("sub").join("b.txt"), b"shared-format\n").unwrap();
    fs::write(src.join("c.txt"), b"will-remove\n").unwrap();

    let store = dir.path().join("store");
    let v1 = dir.path().join("v1.cfdir");
    let v2 = dir.path().join("v2.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    // Content change, add, remove.
    fs::write(src.join("a.txt"), b"hello-format-v2\n").unwrap();
    fs::write(src.join("new.txt"), b"brand-new\n").unwrap();
    fs::remove_file(src.join("c.txt")).unwrap();
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v2.to_str().unwrap(),
        "--seed",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let text_out = run_fail(&["diff", v1.to_str().unwrap(), v2.to_str().unwrap()]);
    assert_eq!(text_out.status.code(), Some(1));
    let text_stdout = String::from_utf8_lossy(&text_out.stdout);
    let text_counts = parse_diff_summary_counts(parse_diff_summary(&text_stdout));

    let json_out = run_fail(&[
        "diff",
        "--format",
        "json",
        v1.to_str().unwrap(),
        v2.to_str().unwrap(),
    ]);
    assert_eq!(
        json_out.status.code(),
        Some(1),
        "json format must keep exit 1 on differences; status={:?}",
        json_out.status
    );
    let json_stdout = String::from_utf8_lossy(&json_out.stdout);
    let json_counts = parse_diff_json_counts(&json_stdout);
    assert_eq!(
        text_counts, json_counts,
        "text vs json counts mismatch\ntext={text_stdout}\njson={json_stdout}"
    );
    assert!(
        text_counts.0 >= 1 && text_counts.1 >= 1 && text_counts.2 >= 1,
        "fixture should exercise added/removed/changed; counts={text_counts:?}"
    );

    // Default (no --format) must match explicit --format text.
    let default_out = run_fail(&["diff", v1.to_str().unwrap(), v2.to_str().unwrap()]);
    let explicit_text = run_fail(&[
        "diff",
        "--format",
        "text",
        v1.to_str().unwrap(),
        v2.to_str().unwrap(),
    ]);
    assert_eq!(
        String::from_utf8_lossy(&default_out.stdout),
        String::from_utf8_lossy(&explicit_text.stdout),
        "default format must ≡ --format text"
    );
}

#[test]
fn diff_format_json_identical_exit_zero() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"same-format\n").unwrap();
    let store = dir.path().join("store");
    let left = dir.path().join("left.cfdir");
    let right = dir.path().join("right.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        left.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        right.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "diff",
        "--format",
        "json",
        left.to_str().unwrap(),
        right.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let counts = parse_diff_json_counts(&stdout);
    assert_eq!(counts.0, 0);
    assert_eq!(counts.1, 0);
    assert_eq!(counts.2, 0);
    assert_eq!(counts.3, 0);
    assert_eq!(counts.5, 0);
    assert_eq!(counts.6, 0);
    assert!(counts.4 >= 1, "expected shared chunks; stdout={stdout}");
}

#[test]
fn diff_format_json_still_rejects_cfidx() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("single.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let cfdir = dir.path().join("tree.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let fail = run_fail(&[
        "diff",
        "--format",
        "json",
        idx.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("cfidx") || err.contains(".cfidx") || err.contains("cfdir"),
        "json format must still reject .cfidx; stderr={err}"
    );
}

fn snapshot_store_files(store: &Path) -> Vec<(String, u64, Vec<u8>)> {
    let chunks = store.join("chunks");
    if !chunks.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for shard in fs::read_dir(&chunks).unwrap() {
        let shard = shard.unwrap().path();
        if !shard.is_dir() {
            continue;
        }
        for ent in fs::read_dir(&shard).unwrap() {
            let p = ent.unwrap().path();
            if p.extension().and_then(|e| e.to_str()) != Some("cnk") {
                continue;
            }
            let meta = fs::metadata(&p).unwrap();
            let bytes = fs::read(&p).unwrap();
            out.push((
                p.strip_prefix(store)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                meta.len(),
                bytes,
            ));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn count_cfdirs(root: &Path) -> usize {
    let mut n = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for ent in fs::read_dir(&dir).unwrap() {
            let p = ent.unwrap().path();
            if p.is_dir() {
                // Skip the store tree; only count sibling/temp .cfdir files.
                if p.file_name().and_then(|s| s.to_str()) == Some("chunks") {
                    continue;
                }
                stack.push(p);
            } else if p.extension().and_then(|e| e.to_str()) == Some("cfdir") {
                n += 1;
            }
        }
    }
    n
}

// --- Phase 7 M4: store scrub ---

#[test]
fn store_scrub_help_lists_flags() {
    let help = run_ok(&["--help"]);
    let help_s = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_s.contains("store"),
        "top-level help should list store:\n{help_s}"
    );

    let s = run_ok(&["store", "--help"]);
    let s_out = String::from_utf8_lossy(&s.stdout);
    assert!(
        s_out.contains("scrub"),
        "store help should list scrub:\n{s_out}"
    );

    let scrub = run_ok(&["store", "scrub", "--help"]);
    let scrub_s = String::from_utf8_lossy(&scrub.stdout);
    assert!(scrub_s.contains("--store"), "{scrub_s}");
    assert!(scrub_s.contains("--jobs"), "{scrub_s}");
    assert!(
        scrub_s.contains("--format"),
        "store scrub --help must list --format:\n{scrub_s}"
    );
    assert!(
        scrub_s.contains("--progress"),
        "store scrub --help must list --progress:\n{scrub_s}"
    );
    assert!(
        scrub_s.contains("--listing"),
        "store scrub --help must list --listing (Phase15 P1):\n{scrub_s}"
    );
}

#[test]
fn store_scrub_healthy_store_ok() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&["store", "scrub", "--store", store.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.lines().any(|l| l.starts_with("scrub: ok=")
            && l.contains("corrupt=0")
            && l.contains("unreadable=0")),
        "expected healthy summary; stdout={stdout}"
    );
    assert!(
        !stdout.contains("scrub: corrupt "),
        "healthy store must not print corrupt lines; stdout={stdout}"
    );
}

#[test]
fn store_scrub_listing_only_referenced_ids() {
    // Phase15 P1: --listing <index> rehashes only referenced ids (local).
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx_a = dir.path().join("a.cfidx");
    let idx_b = dir.path().join("b.cfidx");
    let a = dir.path().join("a.bin");
    let b = dir.path().join("b.bin");
    fs::write(&a, b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    fs::write(&b, b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx_a.to_str().unwrap(),
        a.to_str().unwrap(),
    ]);
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx_b.to_str().unwrap(),
        b.to_str().unwrap(),
    ]);

    // Full scrub sees both blobs' chunks (at least 2).
    let full = run_ok(&[
        "store",
        "scrub",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let full_s = String::from_utf8_lossy(&full.stdout);
    let full_v: serde_json::Value = serde_json::from_str(full_s.trim()).unwrap();
    let full_checked = full_v["checked"].as_u64().unwrap();
    assert!(
        full_checked >= 2,
        "full scrub checked={full_checked}; json={full_s}"
    );

    // Listing a only → fewer (or equal) checked; still healthy.
    let listed = run_ok(&[
        "store",
        "scrub",
        "--store",
        store.to_str().unwrap(),
        "--listing",
        idx_a.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let listed_s = String::from_utf8_lossy(&listed.stdout);
    let listed_v: serde_json::Value = serde_json::from_str(listed_s.trim()).unwrap();
    assert_eq!(listed_v["ok"], true, "listing scrub ok; json={listed_s}");
    assert_eq!(listed_v["corrupt"], 0);
    assert_eq!(listed_v["unreadable"], 0);
    let listed_checked = listed_v["checked"].as_u64().unwrap();
    assert!(
        listed_checked > 0 && listed_checked < full_checked,
        "listing scrub should check a proper subset: listed={listed_checked} full={full_checked}; json={listed_s}"
    );
}

#[test]
fn store_scrub_empty_store_zeros_exit_zero() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    // Create empty store via make of empty fixture, then remove all .cnk? Or
    // open via Store::create. Empty chunks dir after create is fine.
    {
        use chunkforge_store::{Compression, Store};
        Store::create(&store, Compression::None).unwrap();
    }

    let out = run_ok(&["store", "scrub", "--store", store.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout
            .lines()
            .any(|l| l.trim() == "scrub: ok=0 corrupt=0 unreadable=0"),
        "empty store summary; stdout={stdout}"
    );
}

#[test]
fn store_scrub_corrupt_byte_nonzero() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let (cnk_path, hex_id) = first_cnk_id(&store.join("chunks"));
    let mut bytes = fs::read(&cnk_path).unwrap();
    bytes[0] ^= 0xff;
    fs::write(&cnk_path, &bytes).unwrap();

    let fail = run_fail(&["store", "scrub", "--store", store.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&fail.stdout);
    assert!(
        stdout
            .lines()
            .any(|l| l.trim() == format!("scrub: corrupt {hex_id}")),
        "must report corrupt id {hex_id}; stdout={stdout}"
    );
    assert!(
        stdout.lines().any(|l| {
            l.starts_with("scrub: ok=") && l.contains("corrupt=1") && l.contains("unreadable=0")
        }),
        "summary must show corrupt=1; stdout={stdout}"
    );
    // Read-only: file still present after scrub.
    assert!(cnk_path.is_file(), "scrub must not delete corrupt chunk");
}

#[test]
fn store_scrub_unreadable_chunk() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let (cnk_path, hex_id) = first_cnk_id(&store.join("chunks"));
    // Make the chunk unreadable to the current user.
    let mut perms = fs::metadata(&cnk_path).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o000);
    fs::set_permissions(&cnk_path, perms).unwrap();

    let fail = run_fail(&["store", "scrub", "--store", store.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&fail.stdout);
    // Restore perms so tempdir cleanup succeeds.
    let mut restore = fs::metadata(&cnk_path).unwrap().permissions();
    restore.set_mode(0o644);
    let _ = fs::set_permissions(&cnk_path, restore);

    assert!(
        stdout
            .lines()
            .any(|l| l.trim() == format!("scrub: unreadable {hex_id}")),
        "must report unreadable id {hex_id}; stdout={stdout}"
    );
    assert!(
        stdout.lines().any(|l| {
            l.starts_with("scrub: ok=") && l.contains("unreadable=1") && l.contains("corrupt=0")
        }),
        "summary must show unreadable=1; stdout={stdout}"
    );
}

#[test]
fn store_scrub_jobs_flag_accepted() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("binary-256.bin");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "store",
        "scrub",
        "--store",
        store.to_str().unwrap(),
        "--jobs",
        "4",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.lines().any(|l| l.starts_with("scrub: ok=")
            && l.contains("corrupt=0")
            && l.contains("unreadable=0")),
        "jobs=4 healthy scrub; stdout={stdout}"
    );
}

// --- Phase 12 M3: store scrub --format text|json ---

#[test]
fn store_scrub_format_json_healthy_parseable() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "store",
        "scrub",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("scrub: ok=") && !stdout.contains("scrub: corrupt "),
        "json must not emit text scrub lines; stdout={stdout}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("scrub json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["corrupt"].as_u64(), Some(0));
    assert_eq!(v["unreadable"].as_u64(), Some(0));
    let ok_count = v["ok_count"].as_u64().expect("ok_count");
    let checked = v["checked"].as_u64().expect("checked");
    assert!(ok_count >= 1, "ok_count={ok_count}");
    assert_eq!(checked, ok_count);
    assert_eq!(v["corrupt_ids"].as_array().map(|a| a.len()), Some(0));
    assert_eq!(v["unreadable_ids"].as_array().map(|a| a.len()), Some(0));
}

#[test]
fn store_scrub_format_json_corrupt_ids_nonzero_exit() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let (cnk_path, hex_id) = first_cnk_id(&store.join("chunks"));
    let mut bytes = fs::read(&cnk_path).unwrap();
    bytes[0] ^= 0xff;
    fs::write(&cnk_path, &bytes).unwrap();

    let fail = run_fail(&[
        "store",
        "scrub",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        "--jobs",
        "2",
    ]);
    let stdout = String::from_utf8_lossy(&fail.stdout);
    assert!(
        !stdout.contains("scrub: corrupt ") && !stdout.contains("scrub: ok="),
        "json must not emit text scrub lines; stdout={stdout}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("scrub corrupt json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], false);
    assert_eq!(v["corrupt"].as_u64(), Some(1));
    assert_eq!(v["unreadable"].as_u64(), Some(0));
    let ids = v["corrupt_ids"].as_array().expect("corrupt_ids");
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].as_str(), Some(hex_id.as_str()));
    assert_eq!(v["unreadable_ids"].as_array().map(|a| a.len()), Some(0));
    assert!(cnk_path.is_file(), "scrub must not delete corrupt chunk");
}

#[test]
fn store_scrub_default_format_is_text_not_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&["store", "scrub", "--store", store.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.lines().any(|l| l.starts_with("scrub: ok=")),
        "default (no --format) must keep text summary; stdout={stdout}"
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(stdout.trim()).is_err(),
        "default (no --format) stdout must not be pure JSON; got {stdout}"
    );
}

// --- Phase 14 M2: store stats / du + --format json ---

#[test]
fn store_stats_help_lists_stats_du_and_format() {
    let s = run_ok(&["store", "--help"]);
    let s_out = String::from_utf8_lossy(&s.stdout);
    assert!(
        s_out.contains("stats"),
        "store --help should list stats:\n{s_out}"
    );
    assert!(
        s_out.contains("du"),
        "store --help should show du alias:\n{s_out}"
    );

    let stats = run_ok(&["store", "stats", "--help"]);
    let stats_s = String::from_utf8_lossy(&stats.stdout);
    assert!(stats_s.contains("--store"), "{stats_s}");
    assert!(
        stats_s.contains("--format"),
        "store stats --help must list --format:\n{stats_s}"
    );
    assert!(
        stats_s.contains("--decode"),
        "store stats --help must list --decode:\n{stats_s}"
    );
    assert!(
        !stats_s.contains("--apply"),
        "store stats must not offer --apply:\n{stats_s}"
    );

    let du = run_ok(&["store", "du", "--help"]);
    let du_s = String::from_utf8_lossy(&du.stdout);
    assert!(du_s.contains("--store"), "{du_s}");
    assert!(
        du_s.contains("--format"),
        "store du --help must list --format:\n{du_s}"
    );
    assert!(
        du_s.contains("--decode"),
        "store du --help must list --decode:\n{du_s}"
    );
}

#[test]
fn store_stats_empty_store_text_and_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    {
        use chunkforge_store::{Compression, Store};
        Store::create(&store, Compression::None).unwrap();
    }

    let out = run_ok(&["store", "stats", "--store", store.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.lines().any(|l| {
            l.trim() == "store stats: chunks=0 bytes_on_disk=0 bytes_plaintext=0 compression=none"
        }),
        "empty store text summary; stdout={stdout}"
    );

    let out = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("store stats:"),
        "json must not dual-write text; stdout={stdout}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stats json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["chunks"].as_u64(), Some(0));
    assert_eq!(v["bytes_on_disk"].as_u64(), Some(0));
    assert_eq!(v["bytes_plaintext"].as_u64(), Some(0));
    assert_eq!(v["compression"].as_str(), Some("none"));
}

#[test]
fn store_stats_after_put_json_chunks_and_bytes() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("store stats:"),
        "json must not dual-write text; stdout={stdout}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stats json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    let chunks = v["chunks"].as_u64().expect("chunks");
    let bytes = v["bytes_on_disk"].as_u64().expect("bytes_on_disk");
    let plain = v["bytes_plaintext"].as_u64().expect("bytes_plaintext");
    assert!(chunks >= 1, "chunks={chunks}");
    assert!(bytes > 0, "bytes_on_disk={bytes}");
    assert_eq!(plain, bytes, "none-store plaintext ≡ on_disk");
    assert_eq!(v["compression"].as_str(), Some("none"));

    // Cross-check against library + on-disk .cnk sizes
    {
        use chunkforge_store::Store;
        let s = Store::open(&store).unwrap();
        let lib = s.stats().unwrap();
        assert_eq!(chunks, lib.chunks);
        assert_eq!(bytes, lib.bytes_on_disk);
        assert_eq!(lib.bytes_plaintext, Some(bytes));
    }
}

#[test]
fn store_stats_default_format_is_text_not_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&["store", "stats", "--store", store.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.lines().any(|l| l.starts_with("store stats: chunks=")
            && l.contains("bytes_on_disk=")
            && l.contains("bytes_plaintext=")
            && l.contains("compression=")),
        "default (no --format) must keep text summary with bytes_plaintext; stdout={stdout}"
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(stdout.trim()).is_err(),
        "default (no --format) stdout must not be pure JSON; got {stdout}"
    );
}

#[test]
fn store_stats_du_alias_works() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    {
        use chunkforge_store::{Compression, Store};
        Store::create(&store, Compression::None).unwrap();
    }

    let out = run_ok(&["store", "du", "--store", store.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.lines().any(|l| {
            l.trim() == "store stats: chunks=0 bytes_on_disk=0 bytes_plaintext=0 compression=none"
        }),
        "du alias text; stdout={stdout}"
    );

    let out = run_ok(&[
        "store",
        "du",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("du json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["chunks"].as_u64(), Some(0));
    assert_eq!(v["bytes_on_disk"].as_u64(), Some(0));
}

// --- Phase 16 M4: store stats bytes_plaintext + --decode ---

#[test]
fn store_stats_none_json_bytes_plaintext_equals_on_disk() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    {
        use chunkforge_store::{Compression, Store};
        let s = Store::create(&store, Compression::None).unwrap();
        s.put(b"plaintext-a").unwrap();
        s.put(b"plaintext-bb").unwrap();
    }

    let out = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stats json invalid: {e}; stdout={stdout}"));
    let on_disk = v["bytes_on_disk"].as_u64().expect("bytes_on_disk");
    let plain = v["bytes_plaintext"].as_u64().expect("bytes_plaintext");
    assert_eq!(plain, on_disk);
    assert_eq!(v["compression"].as_str(), Some("none"));

    // --decode is a no-op for none stores (still Some ≡ on_disk).
    let out = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        "--decode",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v2: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stats --decode json invalid: {e}; stdout={stdout}"));
    assert_eq!(v2["bytes_plaintext"].as_u64(), Some(on_disk));
    assert_eq!(v2["bytes_on_disk"].as_u64(), Some(on_disk));
}

#[test]
fn store_stats_decode_flag_listed_and_text_prints_plaintext() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--decode",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.lines().any(|l| l.starts_with("store stats:")
            && l.contains("bytes_on_disk=")
            && l.contains("bytes_plaintext=")
            && l.contains("compression=none")),
        "text with --decode must print bytes_plaintext; stdout={stdout}"
    );
}

// --- Phase 7 M6: archive --seed-trust-mtime + extract --force ---

#[test]
fn archive_help_lists_seed_trust_mtime_with_warning() {
    let help = run_ok(&["archive", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--seed-trust-mtime"),
        "archive --help should list --seed-trust-mtime:\n{s}"
    );
    let lower = s.to_lowercase();
    assert!(
        lower.contains("mtime")
            && (lower.contains("warn")
                || lower.contains("forged")
                || lower.contains("risk")
                || lower.contains("miss")),
        "archive --help --seed-trust-mtime should warn about mtime risk:\n{s}"
    );
}

#[test]
fn archive_seed_trust_mtime_requires_seed() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let store = dir.path().join("store");
    let out = dir.path().join("out.cfdir");
    let fail = run_fail(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--seed-trust-mtime",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("seed-trust-mtime")
            || err.contains("--seed")
            || err.to_lowercase().contains("require"),
        "without --seed, --seed-trust-mtime must error; stderr={err}"
    );
}

#[test]
fn archive_seed_trust_mtime_same_tree_reuses() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"trust-a\n").unwrap();
    fs::write(src.join("sub/b.txt"), b"trust-b\n").unwrap();
    let store = dir.path().join("store");
    let v1 = dir.path().join("v1.cfdir");
    let v2 = dir.path().join("v2.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v2.to_str().unwrap(),
        "--seed",
        v1.to_str().unwrap(),
        "--seed-trust-mtime",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("seed_reused_files=2") && err.contains("rechunked_files=0"),
        "unchanged tree + trust-mtime should reuse all; stderr={err}"
    );
}

/// Same size + restored mtime + different content: with `--seed-trust-mtime`
/// → Reuse (documents the risk); without → Rechunk (0.6.0 safety).
#[test]
fn archive_seed_trust_mtime_forged_mtime_misses_content_change() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"AAAAAAAA").unwrap(); // 8 bytes
    fs::write(src.join("b.txt"), b"keep-me\n").unwrap();
    let store = dir.path().join("store");
    let v1 = dir.path().join("v1.cfdir");
    let v2_trust = dir.path().join("v2-trust.cfdir");
    let v2_safe = dir.path().join("v2-safe.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    // Read prior mtime for a.txt from the listing (authoritative for trust).
    let bytes = fs::read(&v1).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).unwrap();
    let prior_mtime = arch
        .entries
        .iter()
        .find_map(|e| match (&e.path[..], &e.kind) {
            ("a.txt", chunkforge_index::DirEntryKind::File { mtime_secs, .. }) => Some(*mtime_secs),
            _ => None,
        })
        .expect("a.txt in prior");

    // Same size, different content; restore mtime to prior so trust would hit.
    fs::write(src.join("a.txt"), b"BBBBBBBB").unwrap();
    restore_mtime_secs(&src.join("a.txt"), prior_mtime);

    let out_trust = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v2_trust.to_str().unwrap(),
        "--seed",
        v1.to_str().unwrap(),
        "--seed-trust-mtime",
        src.to_str().unwrap(),
    ]);
    let err_trust = String::from_utf8_lossy(&out_trust.stderr);
    assert!(
        err_trust.contains("seed_reused_files=2") && err_trust.contains("rechunked_files=0"),
        "trust-mtime + forged mtime must wrongly Reuse dirty a.txt; stderr={err_trust}"
    );

    // Default path (no trust): same dirty tree must Rechunk a.txt.
    let out_safe = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v2_safe.to_str().unwrap(),
        "--seed",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err_safe = String::from_utf8_lossy(&out_safe.stderr);
    assert!(
        err_safe.contains("seed_reused_files=1") && err_safe.contains("rechunked_files=1"),
        "without trust, dirty a.txt must rechunk; stderr={err_safe}"
    );
}

#[test]
fn archive_seed_trust_mtime_mtime_differ_falls_to_blake3() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"stable-bytes\n").unwrap();
    let store = dir.path().join("store");
    let v1 = dir.path().join("v1.cfdir");
    let v2 = dir.path().join("v2.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    // Bump mtime only (content unchanged) → trust cannot short-circuit; blake3 still Reuse.
    restore_mtime_secs(&src.join("a.txt"), 9_999_999_999);

    let out = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v2.to_str().unwrap(),
        "--seed",
        v1.to_str().unwrap(),
        "--seed-trust-mtime",
        "--dry-run",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("would_seed_reuse=1") && err.contains("would_rechunk=0"),
        "mtime differ + same content should blake3-Reuse; stderr={err}"
    );
}

#[test]
fn extract_help_lists_force() {
    let help = run_ok(&["extract", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--force"),
        "extract --help should list --force:\n{s}"
    );
    assert!(
        s.to_lowercase().contains("overwrite") || s.to_lowercase().contains("existing"),
        "extract --help --force should mention overwrite/existing:\n{s}"
    );
}

#[test]
fn extract_force_overwrites_existing_regular_files() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"original-a\n").unwrap();
    fs::write(src.join("sub/b.txt"), b"original-b\n").unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);

    // Corrupt extracted tree, then --force must restore.
    fs::write(out.join("a.txt"), b"DIRTY\n").unwrap();
    fs::write(out.join("sub/b.txt"), b"ALSO-DIRTY\n").unwrap();

    // Without --force still fails.
    let fail = run_fail(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    let fail_err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        fail_err.contains("already exists") || fail_err.contains("refusing"),
        "without --force must refuse; stderr={fail_err}"
    );

    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--force",
    ]);

    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"original-a\n");
    assert_eq!(fs::read(out.join("sub/b.txt")).unwrap(), b"original-b\n");
}

#[test]
fn extract_force_refuses_dir_file_type_mismatch() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"file-payload\n").unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    fs::create_dir_all(&out).unwrap();
    // Place a directory where a.txt (file) should land.
    fs::create_dir_all(out.join("a.txt")).unwrap();

    let fail = run_fail(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--force",
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("directory") && (err.contains("refusing") || err.contains("force")),
        "force must not replace dir with file; stderr={err}"
    );
}

/// Restore `path`'s mtime to `secs` since Unix epoch (Linux `touch -d @secs`).
fn restore_mtime_secs(path: &Path, secs: u64) {
    let status = Command::new("touch")
        .args(["-d", &format!("@{secs}"), path.to_str().expect("utf8 path")])
        .status()
        .expect("spawn touch");
    assert!(
        status.success(),
        "touch -d @{secs} {} failed",
        path.display()
    );
}

// --- Phase 8 M6: CLI `--aws-sigv4` ---

#[test]
fn aws_sigv4_help_listed_on_http_commands() {
    for cmd in ["cat", "verify", "doctor", "push", "extract", "pull"] {
        let out = run_ok(&[cmd, "--help"]);
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(
            s.contains("--aws-sigv4"),
            "{cmd} --help should list --aws-sigv4:\n{s}"
        );
    }
}

#[test]
fn aws_sigv4_without_credentials_errors_clearly() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("t.cfidx");
    let input = dir.path().join("in.bin");
    fs::write(&input, b"phase8-m6-cli-no-creds").unwrap();
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // Ensure credentials are absent for this process invocation (env + shared file).
    let missing_shared = dir.path().join("no-such-aws-credentials");
    let out = Command::new(bin())
        .args([
            "verify",
            "--source",
            "http://127.0.0.1:9",
            "--aws-sigv4",
            idx.to_str().unwrap(),
        ])
        .env_remove("AWS_ACCESS_KEY_ID")
        .env_remove("AWS_SECRET_ACCESS_KEY")
        .env_remove("AWS_SESSION_TOKEN")
        .env("AWS_SHARED_CREDENTIALS_FILE", &missing_shared)
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("AWS_ACCESS_KEY_ID") || err.contains("aws-sigv4"),
        "expected clear credentials error, got: {err}"
    );
}

#[test]
fn aws_sigv4_shared_credentials_file_fallback() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("t.cfidx");
    let input = dir.path().join("in.bin");
    fs::write(&input, b"phase9-m6-shared-creds").unwrap();
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let creds_path = dir.path().join("credentials");
    fs::write(
        &creds_path,
        "[default]\naws_access_key_id = AKIISHARED\naws_secret_access_key = shared-secret\n",
    )
    .unwrap();

    // Env keys absent → shared file must satisfy --aws-sigv4 (connection may fail;
    // credentials error must NOT appear).
    let out = Command::new(bin())
        .args([
            "verify",
            "--source",
            "http://127.0.0.1:9",
            "--aws-sigv4",
            idx.to_str().unwrap(),
        ])
        .env_remove("AWS_ACCESS_KEY_ID")
        .env_remove("AWS_SECRET_ACCESS_KEY")
        .env_remove("AWS_SESSION_TOKEN")
        .env("AWS_SHARED_CREDENTIALS_FILE", &creds_path)
        .env("AWS_REGION", "us-east-1")
        .output()
        .expect("spawn");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !err.contains("requires AWS_ACCESS_KEY_ID"),
        "shared file should supply credentials; got: {err}"
    );
    // Port 9 is closed → backend/connect failure, not missing-creds.
    assert!(!out.status.success());
}

#[test]
fn aws_sigv4_conflicts_with_authorization_header() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("t.cfidx");
    let input = dir.path().join("in.bin");
    fs::write(&input, b"phase8-m6-cli-auth-conflict").unwrap();
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = Command::new(bin())
        .args([
            "verify",
            "--source",
            "http://127.0.0.1:9",
            "--aws-sigv4",
            "--header",
            "Authorization: Bearer tok",
            idx.to_str().unwrap(),
        ])
        .env("AWS_ACCESS_KEY_ID", "AKIATEST")
        .env("AWS_SECRET_ACCESS_KEY", "secret")
        .env("AWS_REGION", "us-east-1")
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.to_ascii_lowercase().contains("authorization") && err.contains("aws-sigv4"),
        "expected Authorization conflict error, got: {err}"
    );
}

#[test]
fn aws_sigv4_push_sends_authorization_header() {
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Seen {
        authorization: Option<String>,
        amz_date: Option<String>,
        put_count: usize,
    }
    let seen: Arc<Mutex<Seen>> = Arc::new(Mutex::new(Seen::default()));
    let seen2 = Arc::clone(&seen);

    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let _handle = thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let method = request.method().clone();
            {
                let mut g = seen2.lock().unwrap();
                for h in request.headers() {
                    let name = h.field.as_str().as_str();
                    if name.eq_ignore_ascii_case("Authorization") {
                        g.authorization = Some(h.value.as_str().to_string());
                    } else if name.eq_ignore_ascii_case("x-amz-date") {
                        g.amz_date = Some(h.value.as_str().to_string());
                    }
                }
                if method == Method::Put || method == Method::Post {
                    g.put_count += 1;
                }
            }
            match method {
                Method::Head => {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
                Method::Put | Method::Post => {
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                    let _ = request.respond(Response::empty(StatusCode(200)));
                }
                _ => {
                    let _ = request.respond(Response::empty(StatusCode(405)));
                }
            }
        }
    });
    thread::sleep(Duration::from_millis(20));

    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("t.cfidx");
    let input = dir.path().join("in.bin");
    fs::write(&input, b"phase8-m6-cli-sigv4-push").unwrap();
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = Command::new(bin())
        .args([
            "push",
            "--store",
            store.to_str().unwrap(),
            "--dest",
            &base,
            "--aws-sigv4",
            idx.to_str().unwrap(),
        ])
        .env("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE")
        .env(
            "AWS_SECRET_ACCESS_KEY",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        )
        .env("AWS_REGION", "us-east-1")
        .output()
        .expect("spawn push");
    assert!(
        out.status.success(),
        "push --aws-sigv4 failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let g = seen.lock().unwrap();
    assert!(g.put_count >= 1, "expected at least one PUT");
    let auth = g
        .authorization
        .as_deref()
        .expect("Authorization header on signed PUT");
    assert!(
        auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"),
        "got {auth}"
    );
    assert!(auth.contains("Signature="), "got {auth}");
    assert!(
        g.amz_date
            .as_ref()
            .is_some_and(|d| d.len() == 16 && d.ends_with('Z')),
        "x-amz-date should look like YYYYMMDDTHHMMSSZ, got {:?}",
        g.amz_date
    );
}

#[test]
fn push_without_aws_sigv4_sends_no_sigv4_headers() {
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Seen {
        authorization: Option<String>,
        amz_date: Option<String>,
        put_count: usize,
    }
    let seen: Arc<Mutex<Seen>> = Arc::new(Mutex::new(Seen::default()));
    let seen2 = Arc::clone(&seen);

    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let _handle = thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let method = request.method().clone();
            {
                let mut g = seen2.lock().unwrap();
                for h in request.headers() {
                    let name = h.field.as_str().as_str();
                    if name.eq_ignore_ascii_case("Authorization") {
                        g.authorization = Some(h.value.as_str().to_string());
                    } else if name.eq_ignore_ascii_case("x-amz-date") {
                        g.amz_date = Some(h.value.as_str().to_string());
                    }
                }
                if method == Method::Put || method == Method::Post {
                    g.put_count += 1;
                }
            }
            match method {
                Method::Head => {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
                Method::Put | Method::Post => {
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                    let _ = request.respond(Response::empty(StatusCode(200)));
                }
                _ => {
                    let _ = request.respond(Response::empty(StatusCode(405)));
                }
            }
        }
    });
    thread::sleep(Duration::from_millis(20));

    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("t.cfidx");
    let input = dir.path().join("in.bin");
    fs::write(&input, b"phase8-m6-cli-no-flag").unwrap();
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // Even with AWS_* set, without --aws-sigv4 there must be no SigV4 headers.
    let out = Command::new(bin())
        .args([
            "push",
            "--store",
            store.to_str().unwrap(),
            "--dest",
            &base,
            idx.to_str().unwrap(),
        ])
        .env("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE")
        .env(
            "AWS_SECRET_ACCESS_KEY",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        )
        .env("AWS_REGION", "us-east-1")
        .output()
        .expect("spawn push");
    assert!(
        out.status.success(),
        "push without --aws-sigv4 failed\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let g = seen.lock().unwrap();
    assert!(g.put_count >= 1);
    assert!(
        g.authorization.is_none(),
        "default off must not send Authorization: {:?}",
        g.authorization
    );
    assert!(
        g.amz_date.is_none(),
        "default off must not send x-amz-date: {:?}",
        g.amz_date
    );
}

// --- Phase 9 M1: extract --skip-unchanged ---

#[test]
fn extract_help_lists_skip_unchanged() {
    let help = run_ok(&["extract", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--skip-unchanged"),
        "extract --help should list --skip-unchanged:\n{s}"
    );
}

// --- Phase 11 M1: extract --skip-trust-mtime ---

#[test]
fn extract_help_lists_skip_trust_mtime_with_warning() {
    let help = run_ok(&["extract", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--skip-trust-mtime"),
        "extract --help should list --skip-trust-mtime:\n{s}"
    );
    let lower = s.to_lowercase();
    assert!(
        lower.contains("mtime")
            && (lower.contains("warn")
                || lower.contains("forged")
                || lower.contains("risk")
                || lower.contains("miss")
                || lower.contains("clock")),
        "extract --help --skip-trust-mtime should warn about mtime risk:\n{s}"
    );
}

#[test]
fn extract_skip_trust_mtime_requires_skip_unchanged() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let out = dir.path().join("out");
    let cfdir = dir.path().join("t.cfdir");
    // Minimal args; clap should fail before needing a real archive.
    let fail = run_fail(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-trust-mtime",
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("skip-trust-mtime")
            || err.contains("skip-unchanged")
            || err.to_lowercase().contains("require"),
        "without --skip-unchanged, --skip-trust-mtime must error; stderr={err}"
    );
}

#[test]
fn extract_skip_unchanged_second_pass_skips_all_and_zero_gets() {
    use std::sync::{Arc, Mutex};

    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"hello-extract-v1\n").unwrap();
    fs::write(src.join("sub/b.txt"), b"payload-b\n").unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("v1.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    // First extract via local store (populate dest tree).
    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);

    let a_before = fs::metadata(out.join("a.txt")).unwrap();
    let b_before = fs::metadata(out.join("sub/b.txt")).unwrap();

    // Counting HTTP GET server over the same store.
    let get_count = Arc::new(Mutex::new(0usize));
    let get_count2 = Arc::clone(&get_count);
    let store_root = store.clone();
    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let _handle = thread::spawn(move || {
        for request in server.incoming_requests() {
            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(&url);
            let rel = path.trim_start_matches('/');
            let file_path = store_root.join(rel);
            if request.method() == &Method::Get {
                *get_count2.lock().unwrap() += 1;
                if file_path.is_file() {
                    let data = fs::read(&file_path).unwrap_or_default();
                    let _ = request.respond(Response::from_data(data));
                } else {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
            } else if request.method() == &Method::Head {
                if file_path.is_file() {
                    let data = fs::read(&file_path).unwrap_or_default();
                    let response = Response::empty(200).with_header(
                        Header::from_bytes(&b"Content-Length"[..], data.len().to_string()).unwrap(),
                    );
                    let _ = request.respond(response);
                } else {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
            } else {
                let _ = request.respond(Response::empty(StatusCode(405)));
            }
        }
    });
    thread::sleep(Duration::from_millis(20));

    let skip_out = run_ok(&[
        "extract",
        "--source",
        &base,
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
        "--force",
    ]);

    let gets = *get_count.lock().unwrap();
    assert_eq!(
        gets, 0,
        "second extract --skip-unchanged must not GET any chunks; gets={gets}"
    );

    let skip_err = String::from_utf8_lossy(&skip_out.stderr);
    assert!(
        skip_err.contains("skipped=2")
            && skip_err.contains("wrote=0")
            && skip_err.contains("dirs="),
        "expected skipped=2 wrote=0 dirs=…; stderr={skip_err}"
    );

    // Files untouched (mtime preserved — whole file not rewritten).
    let a_after = fs::metadata(out.join("a.txt")).unwrap();
    let b_after = fs::metadata(out.join("sub/b.txt")).unwrap();
    assert_eq!(
        a_before.modified().unwrap(),
        a_after.modified().unwrap(),
        "skipped a.txt must keep mtime"
    );
    assert_eq!(
        b_before.modified().unwrap(),
        b_after.modified().unwrap(),
        "skipped b.txt must keep mtime"
    );
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"hello-extract-v1\n");
}

#[test]
fn extract_skip_unchanged_unequal_size_not_skipped() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"short\n").unwrap();
    fs::write(src.join("b.txt"), b"keep-me\n").unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);

    // Unequal size on a.txt → must not skip; without --force still fails.
    fs::write(out.join("a.txt"), b"DIFFERENT-LENGTH-CONTENT\n").unwrap();
    let fail = run_fail(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
    ]);
    let fail_err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        fail_err.contains("already exists") || fail_err.contains("refusing"),
        "unequal size without --force must refuse; stderr={fail_err}"
    );

    // With --force: a.txt rewritten, b.txt skipped (still matches).
    let out2 = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
        "--force",
    ]);
    let err = String::from_utf8_lossy(&out2.stderr);
    assert!(
        err.contains("skipped=1") && err.contains("wrote=1") && err.contains("dirs="),
        "expected skipped=1 wrote=1 dirs=…; stderr={err}"
    );
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"short\n");
    assert_eq!(fs::read(out.join("b.txt")).unwrap(), b"keep-me\n");
}

#[test]
fn extract_skip_unchanged_match_priority_over_force() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"same-content\n").unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);

    let before = fs::metadata(out.join("a.txt")).unwrap();
    let out2 = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
        "--force",
    ]);
    let err = String::from_utf8_lossy(&out2.stderr);
    assert!(
        err.contains("skipped=1") && err.contains("wrote=0") && err.contains("dirs="),
        "match must skip even with --force; stderr={err}"
    );
    let after = fs::metadata(out.join("a.txt")).unwrap();
    assert_eq!(
        before.modified().unwrap(),
        after.modified().unwrap(),
        "matched file must not be rewritten under --force + --skip-unchanged"
    );
}

#[test]
fn extract_without_skip_flag_summary_matches_0_8_0() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let o = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("extract: wrote") && err.contains("1 file") && !err.contains("skipped="),
        "no --skip-unchanged must keep 0.8.0 summary; stderr={err}"
    );
}

// --- Phase 9 M2: --force overlap + summary fields ---

#[test]
fn extract_skip_force_change_one_file_skipped_n_minus_1_wrote_1() {
    // §6.2 C / M2 acceptance: first extract full tree, change 1 source file,
    // re-archive, then extract --skip-unchanged --force → skipped=N-1 wrote=1.
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"hello-extract-v1\n").unwrap();
    fs::write(src.join("sub/b.txt"), b"payload-b\n").unwrap();
    fs::write(src.join("c.txt"), b"keep-c\n").unwrap();
    let store = dir.path().join("store");
    let v1 = dir.path().join("v1.cfdir");
    let v2 = dir.path().join("v2.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        v1.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);

    let b_mtime_before = fs::metadata(out.join("sub/b.txt"))
        .unwrap()
        .modified()
        .unwrap();
    let c_mtime_before = fs::metadata(out.join("c.txt")).unwrap().modified().unwrap();

    // Change exactly one file; re-archive (seed optional but mirrors demo).
    fs::write(src.join("a.txt"), b"hello-extract-v2\n").unwrap();
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        v2.to_str().unwrap(),
        "--seed",
        v1.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let o = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        v2.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
        "--force",
    ]);
    let err = String::from_utf8_lossy(&o.stderr);
    // N=3 files → skipped=2 wrote=1; dirs= present (nailed field names).
    assert!(
        err.contains("skipped=2") && err.contains("wrote=1") && err.contains("dirs="),
        "M2: change 1 of 3 → skipped=2 wrote=1 dirs=…; stderr={err}"
    );
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"hello-extract-v2\n");
    assert_eq!(fs::read(out.join("sub/b.txt")).unwrap(), b"payload-b\n");
    assert_eq!(fs::read(out.join("c.txt")).unwrap(), b"keep-c\n");
    assert_eq!(
        fs::metadata(out.join("sub/b.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        b_mtime_before,
        "unchanged b.txt must not be rewritten"
    );
    assert_eq!(
        fs::metadata(out.join("c.txt")).unwrap().modified().unwrap(),
        c_mtime_before,
        "unchanged c.txt must not be rewritten"
    );
}

#[test]
fn extract_skip_content_mismatch_requires_force_then_writes() {
    // Same size, different bytes: without --force fails (≡ 0.8.0); with --force writes.
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"AAAA").unwrap();
    fs::write(src.join("b.txt"), b"BBBB").unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);

    // Same length, different content on a.txt.
    fs::write(out.join("a.txt"), b"XXXX").unwrap();
    let fail = run_fail(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
    ]);
    let fail_err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        fail_err.contains("already exists") || fail_err.contains("refusing"),
        "content mismatch without --force must refuse; stderr={fail_err}"
    );

    let o = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
        "--force",
    ]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("skipped=1") && err.contains("wrote=1") && err.contains("dirs="),
        "content mismatch + force → skipped=1 wrote=1; stderr={err}"
    );
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"AAAA");
    assert_eq!(fs::read(out.join("b.txt")).unwrap(), b"BBBB");
}

#[test]
fn extract_help_force_mentions_skip_match_priority() {
    let help = run_ok(&["extract", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--force") && s.contains("--skip-unchanged"),
        "extract --help should list --force and --skip-unchanged:\n{s}"
    );
}

// --- Phase 9 M3: extract --dry-run ---

#[test]
fn extract_help_lists_dry_run() {
    let help = run_ok(&["extract", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--dry-run"),
        "extract --help should list --dry-run:\n{s}"
    );
}

#[test]
fn extract_dry_run_no_skip_no_writes_and_no_store_access() {
    // Without --skip-unchanged: no source/store open; would_write=all files;
    // create/modify nothing under -o (including output root).
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"hello-dry\n").unwrap();
    fs::write(src.join("sub/b.txt"), b"payload-b\n").unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("v1.cfdir");
    let out = dir.path().join("out-missing");
    // Bogus store path that must NOT be opened/created by dry-run.
    let missing_store = dir.path().join("store-does-not-exist");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    assert!(!out.exists(), "out must start missing");
    assert!(!missing_store.exists());

    let o = run_ok(&[
        "extract",
        "--store",
        missing_store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--dry-run",
    ]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("extract: dry-run:")
            && err.contains("would_skip=0")
            && err.contains("would_write=2")
            && err.contains("would_dirs=")
            && err.contains("would_fail=0"),
        "expected dry-run would_skip=0 would_write=2 would_dirs=… would_fail=0; stderr={err}"
    );
    assert!(!out.exists(), "dry-run must not create output root");
    assert!(
        !missing_store.exists(),
        "dry-run without --skip-unchanged must not open/create --store"
    );
}

#[test]
fn extract_dry_run_with_skip_reads_local_only_zero_gets() {
    use std::sync::{Arc, Mutex};

    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"hello-extract-v1\n").unwrap();
    fs::write(src.join("sub/b.txt"), b"payload-b\n").unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("v1.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);

    let a_before = fs::metadata(out.join("a.txt")).unwrap();
    let b_before = fs::metadata(out.join("sub/b.txt")).unwrap();
    let a_bytes = fs::read(out.join("a.txt")).unwrap();
    let b_bytes = fs::read(out.join("sub/b.txt")).unwrap();

    let get_count = Arc::new(Mutex::new(0usize));
    let get_count2 = Arc::clone(&get_count);
    let store_root = store.clone();
    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let _handle = thread::spawn(move || {
        for request in server.incoming_requests() {
            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(&url);
            let rel = path.trim_start_matches('/');
            let file_path = store_root.join(rel);
            if request.method() == &Method::Get {
                *get_count2.lock().unwrap() += 1;
                if file_path.is_file() {
                    let data = fs::read(&file_path).unwrap_or_default();
                    let _ = request.respond(Response::from_data(data));
                } else {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
            } else if request.method() == &Method::Head {
                if file_path.is_file() {
                    let data = fs::read(&file_path).unwrap_or_default();
                    let response = Response::empty(200).with_header(
                        Header::from_bytes(&b"Content-Length"[..], data.len().to_string()).unwrap(),
                    );
                    let _ = request.respond(response);
                } else {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
            } else {
                let _ = request.respond(Response::empty(StatusCode(405)));
            }
        }
    });
    thread::sleep(Duration::from_millis(20));

    let o = run_ok(&[
        "extract",
        "--source",
        &base,
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
        "--dry-run",
    ]);
    let gets = *get_count.lock().unwrap();
    assert_eq!(
        gets, 0,
        "dry-run + skip must not GET any chunks; gets={gets}"
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("extract: dry-run:")
            && err.contains("would_skip=2")
            && err.contains("would_write=0")
            && err.contains("would_dirs=")
            && err.contains("would_fail=0"),
        "expected would_skip=2 would_write=0; stderr={err}"
    );

    // Tree untouched.
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), a_bytes);
    assert_eq!(fs::read(out.join("sub/b.txt")).unwrap(), b_bytes);
    assert_eq!(
        a_before.modified().unwrap(),
        fs::metadata(out.join("a.txt")).unwrap().modified().unwrap()
    );
    assert_eq!(
        b_before.modified().unwrap(),
        fs::metadata(out.join("sub/b.txt"))
            .unwrap()
            .modified()
            .unwrap()
    );
}

#[test]
fn extract_dry_run_mismatch_would_fail_without_force_would_write_with_force() {
    // Existing mismatch: no force → would_fail (exit 0); with force → would_write.
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"AAAA").unwrap();
    fs::write(src.join("b.txt"), b"BBBB").unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);

    // Same length, different content on a.txt; b.txt still matches.
    fs::write(out.join("a.txt"), b"XXXX").unwrap();
    let dirty = fs::read(out.join("a.txt")).unwrap();

    let no_force = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
        "--dry-run",
    ]);
    let err = String::from_utf8_lossy(&no_force.stderr);
    assert!(
        err.contains("would_skip=1")
            && err.contains("would_write=0")
            && err.contains("would_fail=1")
            && err.contains("would_dirs="),
        "mismatch without --force → would_fail=1 would_skip=1; stderr={err}"
    );
    assert_eq!(
        fs::read(out.join("a.txt")).unwrap(),
        dirty,
        "dry-run must not rewrite mismatched file"
    );

    let with_force = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
        "--force",
        "--dry-run",
    ]);
    let err2 = String::from_utf8_lossy(&with_force.stderr);
    assert!(
        err2.contains("would_skip=1")
            && err2.contains("would_write=1")
            && err2.contains("would_fail=0")
            && err2.contains("would_dirs="),
        "mismatch + --force → would_write=1 would_skip=1; stderr={err2}"
    );
    assert_eq!(
        fs::read(out.join("a.txt")).unwrap(),
        dirty,
        "dry-run + force must still not write"
    );
}

#[test]
fn extract_dry_run_without_skip_existing_conflict_would_fail() {
    // No skip: existing dest without --force → would_fail (presence only; no hash).
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"content-a\n").unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);

    let before = fs::read(out.join("a.txt")).unwrap();
    let o = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--dry-run",
    ]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("would_skip=0")
            && err.contains("would_write=0")
            && err.contains("would_fail=1")
            && err.contains("would_dirs="),
        "no-skip dry-run + existing → would_fail=1; stderr={err}"
    );
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), before);

    let o2 = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        cfdir.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--force",
        "--dry-run",
    ]);
    let err2 = String::from_utf8_lossy(&o2.stderr);
    assert!(
        err2.contains("would_write=1") && err2.contains("would_fail=0"),
        "no-skip dry-run + force → would_write=1; stderr={err2}"
    );
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), before);
}

#[test]
fn extract_dry_run_invalid_listing_nonzero() {
    let dir = tempdir().unwrap();
    let bad = dir.path().join("not-a-cfdir.bin");
    fs::write(&bad, b"not-a-listing").unwrap();
    let out = dir.path().join("out");
    let missing_store = dir.path().join("no-store");
    let fail = run_fail(&[
        "extract",
        "--store",
        missing_store.to_str().unwrap(),
        bad.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(!fail.status.success());
    assert!(!out.exists(), "failed dry-run must not create output");
}

// --- Phase 10 M6 P1 O1: verify / doctor --format json ---

#[test]
fn verify_format_json_cfidx_ok() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let text_out = run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let text_err = String::from_utf8_lossy(&text_out.stderr);
    assert!(
        text_err.contains("verify: ok"),
        "default text must keep stderr summary; stderr={text_err}"
    );

    let json_out = run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&json_out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("verify json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["kind"], "cfidx");
    assert!(v["bytes"].as_u64().unwrap() > 0, "bytes={v}");
    assert!(v["chunks"].as_u64().unwrap() >= 1, "chunks={v}");
    assert!(
        v.get("cache_hits").is_none()
            && v.get("cache_miss_fills").is_none()
            && v.get("cache_miss_refused").is_none(),
        "verify without --cache must omit cache_*; got {v}"
    );
}

#[test]
fn verify_help_lists_format() {
    let d = run_ok(&["verify", "--help"]);
    let s = String::from_utf8_lossy(&d.stdout);
    assert!(
        s.contains("--format"),
        "verify --help should list --format:\n{s}"
    );
}

#[test]
fn doctor_format_json_ok_and_missing() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let ok = run_ok(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&ok.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("doctor json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["listings"], 1);
    assert_eq!(v["missing"], 0);
    assert_eq!(v["deep"], false);
    assert!(v["checked"].as_u64().unwrap() >= 1, "checked={v}");

    // Empty store → missing; json must carry ids, not bare hex lines alone.
    let empty = dir.path().join("empty-store");
    fs::create_dir_all(&empty).unwrap();
    // Need a valid store layout: make into empty then wipe chunks, or open via Store.
    // Simpler: doctor against a fresh empty dir after Store::open via make of another file then delete chunks.
    let store2 = dir.path().join("store2");
    let idx2 = dir.path().join("other.cfidx");
    run_ok(&[
        "make",
        "--store",
        store2.to_str().unwrap(),
        "-o",
        idx2.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    // Remove all chunk files under store2 so presence fails.
    let chunks_root = store2.join("chunks");
    if chunks_root.exists() {
        fs::remove_dir_all(&chunks_root).unwrap();
        fs::create_dir_all(&chunks_root).unwrap();
    }

    let fail = run_fail(&[
        "doctor",
        "--store",
        store2.to_str().unwrap(),
        "--format",
        "json",
        idx2.to_str().unwrap(),
    ]);
    let fail_stdout = String::from_utf8_lossy(&fail.stdout);
    let fv: serde_json::Value = serde_json::from_str(fail_stdout.trim())
        .unwrap_or_else(|e| panic!("doctor missing json invalid: {e}; stdout={fail_stdout}"));
    assert_eq!(fv["ok"], false);
    let missing = fv["missing"]
        .as_array()
        .unwrap_or_else(|| panic!("missing must be array; {fv}"));
    assert!(!missing.is_empty(), "expected missing ids; {fv}");
    // Bare hex lines must NOT appear outside JSON (stdout should be one JSON object).
    let trimmed = fail_stdout.trim();
    assert!(
        trimmed.starts_with('{') && trimmed.ends_with('}'),
        "json mode must not print bare missing ids; stdout={fail_stdout}"
    );
}

// --- Phase 11 M2: extract --format json ---

#[test]
fn extract_help_lists_format() {
    let help = run_ok(&["extract", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--format"),
        "extract --help should list --format:\n{s}"
    );
    assert!(
        s.contains("text") && s.contains("json"),
        "extract --help --format should mention text|json:\n{s}"
    );
}

#[test]
fn extract_format_json_write_path_ok() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let src = dir.path().join("src");
    let archive = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"hello-extract-json\n").unwrap();

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    // Default text: stderr summary, stdout empty-ish.
    let text = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        archive.to_str().unwrap(),
    ]);
    let text_err = String::from_utf8_lossy(&text.stderr);
    assert!(
        text_err.contains("extract: wrote"),
        "default text must keep stderr summary; stderr={text_err}"
    );

    // Fresh out2 with --format json.
    let out2 = dir.path().join("out2");
    let json_out = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out2.to_str().unwrap(),
        "--format",
        "json",
        archive.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&json_out.stdout);
    let stderr = String::from_utf8_lossy(&json_out.stderr);
    assert!(
        !stderr.contains("extract: wrote") && !stderr.contains("skipped="),
        "json mode must not duplicate summary on stderr; stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("extract json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["dry_run"], false);
    assert_eq!(v["skipped"], 0);
    assert!(v["wrote"].as_u64().unwrap() >= 1, "wrote={v}");
    assert!(v.get("dirs").is_some(), "dirs missing: {v}");
    assert!(
        v.get("cache_hits").is_none()
            && v.get("cache_miss_fills").is_none()
            && v.get("cache_miss_refused").is_none(),
        "extract without --cache must omit cache_*; got {v}"
    );
    assert_eq!(
        fs::read(out2.join("a.txt")).unwrap(),
        b"hello-extract-json\n"
    );
}

#[test]
fn extract_format_json_dry_run() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let src = dir.path().join("src");
    let archive = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"dry-run-json\n").unwrap();
    fs::create_dir_all(src.join("sub")).unwrap();

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let json_out = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--dry-run",
        "--format",
        "json",
        archive.to_str().unwrap(),
    ]);
    assert!(!out.exists(), "dry-run must not create output root");
    let stdout = String::from_utf8_lossy(&json_out.stdout);
    let stderr = String::from_utf8_lossy(&json_out.stderr);
    assert!(
        !stderr.contains("dry-run:"),
        "json dry-run must not duplicate summary on stderr; stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("extract dry-run json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["would_skip"], 0);
    assert!(v["would_write"].as_u64().unwrap() >= 1, "would_write={v}");
    assert!(v.get("would_dirs").is_some(), "would_dirs missing: {v}");
    assert_eq!(v["would_fail"], 0);
}

#[test]
fn extract_format_json_skip_unchanged_counts() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let src = dir.path().join("src");
    let archive = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"skip-json-a\n").unwrap();
    fs::write(src.join("b.txt"), b"skip-json-b\n").unwrap();

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        archive.to_str().unwrap(),
    ]);

    let json_out = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
        "--format",
        "json",
        archive.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&json_out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("extract skip json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["skipped"], 2);
    assert_eq!(v["wrote"], 0);
    assert!(v.get("dirs").is_some(), "dirs missing: {v}");
}

// --- Phase 11 M3: push / pull --format json ---

#[test]
fn push_help_lists_format() {
    let help = run_ok(&["push", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--format"),
        "push --help should list --format:\n{s}"
    );
    assert!(
        s.contains("text") && s.contains("json"),
        "push --help --format should mention text|json:\n{s}"
    );
}

#[test]
fn pull_help_lists_format() {
    let help = run_ok(&["pull", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--format"),
        "pull --help should list --format:\n{s}"
    );
    assert!(
        s.contains("text") && s.contains("json"),
        "pull --help --format should mention text|json:\n{s}"
    );
}

#[test]
fn push_format_json_ok_fields() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    // Default text keeps stderr summary.
    let text = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        "--dry-run",
        idx.to_str().unwrap(),
    ]);
    let text_err = String::from_utf8_lossy(&text.stderr);
    assert!(
        text_err.contains("push: skipped=") || text_err.contains("uploaded="),
        "default text must keep stderr summary; stderr={text_err}"
    );

    let json_out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&json_out.stdout);
    let stderr = String::from_utf8_lossy(&json_out.stderr);
    assert!(
        !stderr.contains("push: skipped=") && !stderr.contains("uploaded="),
        "json mode must not duplicate summary on stderr; stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("push json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["failed"], 0);
    assert_eq!(v["failed_transient"], 0);
    assert_eq!(v["failed_permanent"], 0);
    assert_eq!(v["dry_run"], false);
    assert_eq!(v["listings"], 1);
    assert!(
        v["unique_chunks"].as_u64().unwrap() >= 1,
        "unique_chunks={v}"
    );
    assert!(v["uploaded"].as_u64().unwrap() >= 1, "uploaded={v}");
    assert!(v.get("skipped").is_some(), "skipped missing: {v}");
    assert!(v.get("retries").is_some(), "retries missing: {v}");
    assert!(
        put_count.load(Ordering::SeqCst) >= 1,
        "expected at least one PUT"
    );
}

#[test]
fn push_format_json_dry_run() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));

    let json_out = run_ok(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        "--dry-run",
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    assert_eq!(put_count.load(Ordering::SeqCst), 0, "dry-run must not PUT");
    let stdout = String::from_utf8_lossy(&json_out.stdout);
    let stderr = String::from_utf8_lossy(&json_out.stderr);
    assert!(
        !stderr.contains("push: skipped="),
        "json dry-run must not duplicate summary; stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("push dry-run json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["failed"], 0);
    assert!(v["uploaded"].as_u64().unwrap() >= 1, "uploaded={v}");
}

#[test]
fn push_format_json_failed_ok_false() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // Delete all local chunks so push fails permanently.
    let chunks = local.join("chunks");
    if chunks.is_dir() {
        fs::remove_dir_all(&chunks).unwrap();
        fs::create_dir_all(&chunks).unwrap();
    }

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));

    let out = run_fail(&[
        "push",
        "--store",
        local.to_str().unwrap(),
        "--dest",
        &base,
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("push: fail "),
        "json mode should omit per-id fail lines; stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("push fail json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], false);
    assert!(v["failed"].as_u64().unwrap() >= 1, "failed={v}");
    assert!(
        v["failed_permanent"].as_u64().unwrap() >= 1,
        "failed_permanent={v}"
    );
    assert_eq!(v["failed_transient"], 0);
}

#[test]
fn pull_format_json_ok_fields() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // Default text.
    let text = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--dry-run",
        idx.to_str().unwrap(),
    ]);
    let text_err = String::from_utf8_lossy(&text.stderr);
    assert!(
        text_err.contains("pull: skipped=") || text_err.contains("fetched="),
        "default text must keep stderr summary; stderr={text_err}"
    );

    let json_out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&json_out.stdout);
    let stderr = String::from_utf8_lossy(&json_out.stderr);
    assert!(
        !stderr.contains("pull: skipped=") && !stderr.contains("fetched="),
        "json mode must not duplicate summary on stderr; stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("pull json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["failed"], 0);
    assert_eq!(v["failed_transient"], 0);
    assert_eq!(v["failed_permanent"], 0);
    assert_eq!(v["dry_run"], false);
    assert_eq!(v["listings"], 1);
    assert!(
        v["unique_chunks"].as_u64().unwrap() >= 1,
        "unique_chunks={v}"
    );
    assert!(v["fetched"].as_u64().unwrap() >= 1, "fetched={v}");
    assert!(v.get("skipped").is_some(), "skipped missing: {v}");
    assert!(v.get("retries").is_some(), "retries missing: {v}");
    // No `uploaded` on pull.
    assert!(
        v.get("uploaded").is_none(),
        "pull must use fetched not uploaded: {v}"
    );
    assert!(
        v.get("cache_hits").is_none()
            && v.get("cache_miss_fills").is_none()
            && v.get("cache_miss_refused").is_none(),
        "pull without --cache must omit cache_*; got {v}"
    );

    run_ok(&[
        "verify",
        "--store",
        newstore.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
}

#[test]
fn pull_format_json_dry_run() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let json_out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--dry-run",
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    assert!(
        !newstore.join("meta.toml").exists(),
        "dry-run must not create store"
    );
    let stdout = String::from_utf8_lossy(&json_out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("pull dry-run json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["failed"], 0);
    assert!(v["fetched"].as_u64().unwrap() >= 1, "fetched={v}");
}

// --- Phase 12 M6: opt-in --progress (push / pull / scrub / gc) ---

#[test]
fn progress_help_listed_on_push_pull_scrub_gc() {
    let cases: &[(&[&str], &str)] = &[
        (&["push", "--help"], "push"),
        (&["pull", "--help"], "pull"),
        (&["store", "scrub", "--help"], "store scrub"),
        (&["gc", "--help"], "gc"),
    ];
    for (args, label) in cases {
        let help = run_ok(args);
        let s = String::from_utf8_lossy(&help.stdout);
        assert!(
            s.contains("--progress"),
            "{label} --help must list --progress:\n{s}"
        );
    }
}

#[test]
fn store_scrub_progress_emits_stderr_lines() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let with = run_ok(&[
        "store",
        "scrub",
        "--store",
        store.to_str().unwrap(),
        "--progress",
    ]);
    let err = String::from_utf8_lossy(&with.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=scrub done=")),
        "with --progress stderr must contain progress: lines; stderr={err}"
    );
    // Total known → done=N/TOTAL form
    assert!(
        err.lines().any(|l| l.contains("done=") && l.contains('/')),
        "scrub progress should include done=N/TOTAL; stderr={err}"
    );
    let out = String::from_utf8_lossy(&with.stdout);
    assert!(
        out.lines().any(|l| l.starts_with("scrub: ok=")),
        "text summary still on stdout; stdout={out}"
    );
    assert!(
        !out.contains("progress:"),
        "progress must not pollute stdout; stdout={out}"
    );

    let without = run_ok(&["store", "scrub", "--store", store.to_str().unwrap()]);
    let err0 = String::from_utf8_lossy(&without.stderr);
    assert!(
        !err0.contains("progress:"),
        "without --progress stderr must not contain progress:; stderr={err0}"
    );
}

#[test]
fn store_scrub_progress_orthogonal_to_format_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "store",
        "scrub",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        "--progress",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("progress: op=scrub"),
        "progress on stderr with json; stderr={stderr}"
    );
    assert!(
        !stdout.contains("progress:"),
        "json stdout must not contain progress:; stdout={stdout}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("scrub json invalid with --progress: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert!(v["checked"].as_u64().unwrap() >= 1);
}

#[test]
fn push_progress_with_stub_emits_and_default_silent() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");
    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror.clone(), Arc::clone(&put_count));

    let with = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base,
        "--progress",
        "--dry-run",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&with.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=push done=")),
        "push --progress stderr; stderr={err}"
    );
    assert!(
        err.contains("push:"),
        "text summary still on stderr; stderr={err}"
    );

    let without = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base,
        "--dry-run",
        idx.to_str().unwrap(),
    ]);
    let err0 = String::from_utf8_lossy(&without.stderr);
    assert!(
        !err0.contains("progress:"),
        "push without --progress must be silent on progress:; stderr={err0}"
    );
}

// --- Phase 13 M2: archive --path/--exclude + --format json ---

#[test]
fn archive_help_lists_path_exclude_format() {
    let help = run_ok(&["archive", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--path"),
        "archive --help should list --path:\n{s}"
    );
    assert!(
        s.contains("--exclude"),
        "archive --help should list --exclude:\n{s}"
    );
    assert!(
        s.contains("--format"),
        "archive --help should list --format:\n{s}"
    );
}

#[test]
fn archive_exclude_omit_junk_from_cfdir() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("pkg")).unwrap();
    fs::create_dir_all(src.join(".git")).unwrap();
    fs::create_dir_all(src.join("junk")).unwrap();
    fs::write(src.join("pkg").join("a.txt"), b"keep-me\n").unwrap();
    fs::write(src.join(".git").join("config"), b"ign\n").unwrap();
    fs::write(src.join("junk").join("noise.txt"), b"noise\n").unwrap();

    let store = dir.path().join("store");
    let out = dir.path().join("app.cfdir");
    let result = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--exclude",
        ".git/",
        "--exclude",
        "junk/",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&result.stderr);
    assert!(
        err.contains("excluded=") && !err.contains("excluded=0"),
        "stderr should report excluded≥1; stderr={err}"
    );

    let bytes = fs::read(&out).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).expect("decode .cfdir");
    let paths: Vec<_> = arch.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, vec!["pkg/a.txt"], "paths={paths:?}");
    assert!(
        !paths
            .iter()
            .any(|p| p.starts_with(".git") || p.starts_with("junk")),
        "listing must omit junk; paths={paths:?}"
    );
}

#[test]
fn archive_format_json_dry_run_and_write_parseable() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"json-a\n").unwrap();
    fs::write(src.join("sub").join("b.txt"), b"json-b\n").unwrap();
    fs::write(src.join("skip.o"), b"obj\n").unwrap();

    let store = dir.path().join("store");
    let out = dir.path().join("app.cfdir");

    let dry = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--exclude",
        "*.o",
        "--dry-run",
        "--format",
        "json",
        src.to_str().unwrap(),
    ]);
    let dry_stdout = String::from_utf8_lossy(&dry.stdout);
    let dry_stderr = String::from_utf8_lossy(&dry.stderr);
    assert!(
        !dry_stderr.contains("archive: dry-run:"),
        "json must not dual-write text summary; stderr={dry_stderr}"
    );
    let dry_v: serde_json::Value =
        serde_json::from_str(dry_stdout.trim()).expect("dry-run json parse");
    assert_eq!(dry_v["ok"], true);
    assert_eq!(dry_v["dry_run"], true);
    assert!(dry_v.get("would_write").is_some(), "{dry_v}");
    assert!(dry_v.get("would_reuse").is_some(), "{dry_v}");
    assert!(
        dry_v.get("written").is_none(),
        "dry-run must use would_*; {dry_v}"
    );
    assert!(dry_v["excluded"].as_u64().unwrap_or(0) >= 1, "{dry_v}");
    assert!(!out.exists(), "dry-run must not write .cfdir");

    let wrote = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--exclude",
        "*.o",
        "--format",
        "json",
        src.to_str().unwrap(),
    ]);
    let wrote_stdout = String::from_utf8_lossy(&wrote.stdout);
    let wrote_stderr = String::from_utf8_lossy(&wrote.stderr);
    assert!(
        !wrote_stderr.contains("archive: wrote"),
        "json must not dual-write text summary; stderr={wrote_stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(wrote_stdout.trim()).expect("write json parse");
    assert_eq!(v["ok"], true);
    assert_eq!(v["dry_run"], false);
    assert!(v.get("written").is_some(), "{v}");
    assert!(v.get("reused").is_some(), "{v}");
    assert!(
        v.get("would_write").is_none(),
        "normal write must use written/reused; {v}"
    );
    for key in [
        "files",
        "dirs",
        "chunks",
        "seed_reused_files",
        "rechunked_files",
        "skipped_symlinks",
        "skipped_special",
        "excluded",
    ] {
        assert!(v.get(key).is_some(), "missing {key} in {v}");
    }
    assert!(v["excluded"].as_u64().unwrap_or(0) >= 1, "{v}");
    assert_eq!(v["files"].as_u64().unwrap(), 2, "{v}");

    let bytes = fs::read(&out).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).expect("decode");
    let paths: Vec<_> = arch.entries.iter().map(|e| e.path.as_str()).collect();
    assert!(!paths.iter().any(|p| p.ends_with(".o")), "{paths:?}");
}

#[test]
fn archive_default_format_is_text() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"text-default\n").unwrap();
    let store = dir.path().join("store");
    let out = dir.path().join("a.cfdir");
    let result = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("archive: wrote"),
        "default format ≡ text summary on stderr; stderr={stderr}"
    );
    assert!(
        stdout.trim().is_empty() || !stdout.trim().starts_with('{'),
        "default must not emit JSON on stdout; stdout={stdout}"
    );
}

#[test]
fn archive_no_filter_flags_full_tree_regression() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"full-a\n").unwrap();
    fs::write(src.join("sub").join("b.txt"), b"full-b\n").unwrap();
    let store = dir.path().join("store");
    let out = dir.path().join("full.cfdir");
    let result = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&result.stderr);
    assert!(err.contains("excluded=0"), "stderr={err}");
    let bytes = fs::read(&out).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).expect("decode");
    let mut paths: Vec<_> = arch.entries.iter().map(|e| e.path.clone()).collect();
    paths.sort();
    assert_eq!(paths, vec!["a.txt".to_string(), "sub/b.txt".to_string()]);
}

#[test]
fn archive_path_include_then_exclude() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("pkg").join("foo")).unwrap();
    fs::create_dir_all(src.join("pkg").join("bar")).unwrap();
    fs::create_dir_all(src.join("other")).unwrap();
    fs::write(src.join("pkg").join("foo").join("a.txt"), b"foo\n").unwrap();
    fs::write(src.join("pkg").join("bar").join("b.txt"), b"bar\n").unwrap();
    fs::write(src.join("other").join("c.txt"), b"other\n").unwrap();

    let store = dir.path().join("store");
    let out = dir.path().join("scoped.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--path",
        "pkg",
        "--exclude",
        "pkg/bar/",
        src.to_str().unwrap(),
    ]);
    let bytes = fs::read(&out).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).expect("decode");
    let paths: Vec<_> = arch.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, vec!["pkg/foo/a.txt"], "{paths:?}");
}

#[test]
fn archive_illegal_exclude_errors_clearly() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let store = dir.path().join("store");
    let out = dir.path().join("bad.cfdir");
    let fail = run_fail(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--exclude",
        "a*b",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("invalid exclude pattern") || err.contains("exclude pattern"),
        "stderr={err}"
    );
    assert!(!out.exists());
}

// --- Phase 13 M3: extract --path/--exclude (non-prune) ---

#[test]
fn extract_help_lists_path_exclude_no_delete() {
    let help = run_ok(&["extract", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--path"),
        "extract --help should list --path:\n{s}"
    );
    assert!(
        s.contains("--exclude"),
        "extract --help should list --exclude:\n{s}"
    );
    // Flag listing uses leading spaces + `--name`; about text must not advertise a delete mode.
    assert!(
        !s.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("--delete") || t.starts_with("-d, --delete")
        }),
        "extract --help must NOT list a --delete flag (non-prune):\n{s}"
    );
}

#[test]
fn extract_path_subset_does_not_prune_extra_dest() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("packages").join("foo")).unwrap();
    fs::create_dir_all(src.join("packages").join("bar")).unwrap();
    fs::create_dir_all(src.join("other")).unwrap();
    fs::write(src.join("packages").join("foo").join("a.txt"), b"foo-a\n").unwrap();
    fs::write(src.join("packages").join("bar").join("b.txt"), b"bar-b\n").unwrap();
    fs::write(src.join("other").join("c.txt"), b"other-c\n").unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("full.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = dir.path().join("out");
    fs::create_dir_all(&out).unwrap();
    // Pre-seed an unselected path that must survive subset extract (non-prune).
    let preset = out.join("preset-extra.txt");
    fs::write(&preset, b"i-must-remain\n").unwrap();
    // Also pre-seed a path that exists in the listing but will be filtered out.
    fs::create_dir_all(out.join("packages").join("bar")).unwrap();
    let filtered_existing = out.join("packages").join("bar").join("b.txt");
    fs::write(&filtered_existing, b"stale-bar\n").unwrap();

    let result = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--path",
        "packages/foo",
        "--force",
        listing.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&result.stderr);
    assert!(
        err.contains("1 file") || err.contains("wrote=1") || err.contains("(1 file"),
        "subset extract should write the one matching file; stderr={err}"
    );

    let foo = out.join("packages").join("foo").join("a.txt");
    assert!(
        foo.is_file(),
        "subset file should be written: {}",
        foo.display()
    );
    assert_eq!(fs::read(&foo).unwrap(), b"foo-a\n");

    // Non-prune: preset extra and filtered-out listing path must remain.
    assert!(
        preset.is_file(),
        "preset unselected dest file must survive (non-prune)"
    );
    assert_eq!(fs::read(&preset).unwrap(), b"i-must-remain\n");
    assert!(
        filtered_existing.is_file(),
        "filtered-out listing path must not be deleted"
    );
    assert_eq!(
        fs::read(&filtered_existing).unwrap(),
        b"stale-bar\n",
        "filtered-out path content must be untouched"
    );

    // Unselected listing siblings must not be written.
    assert!(
        !out.join("other").join("c.txt").exists(),
        "unselected listing path must not be materialized"
    );
}

#[test]
fn extract_exclude_skips_junk_dry_run_json_orthogonal() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("pkg")).unwrap();
    fs::create_dir_all(src.join("junk")).unwrap();
    fs::write(src.join("pkg").join("keep.txt"), b"keep\n").unwrap();
    fs::write(src.join("junk").join("noise.txt"), b"noise\n").unwrap();
    fs::write(src.join("skip.o"), b"obj\n").unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("app.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = dir.path().join("out");
    let dry = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--exclude",
        "junk/",
        "--exclude",
        "*.o",
        "--dry-run",
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let dry_stdout = String::from_utf8_lossy(&dry.stdout);
    let dry_stderr = String::from_utf8_lossy(&dry.stderr);
    assert!(
        !dry_stderr.contains("extract: dry-run:"),
        "json must not dual-write text summary; stderr={dry_stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(dry_stdout.trim()).expect("dry-run json parse");
    assert_eq!(v["ok"], true);
    assert_eq!(v["dry_run"], true);
    assert_eq!(
        v["would_write"].as_u64().unwrap(),
        1,
        "only pkg/keep.txt should would_write; got {v}"
    );
    assert!(!out.exists(), "dry-run must not create output root");

    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--exclude",
        "junk/",
        "--exclude",
        "*.o",
        listing.to_str().unwrap(),
    ]);
    assert!(out.join("pkg").join("keep.txt").is_file());
    assert!(!out.join("junk").join("noise.txt").exists());
    assert!(!out.join("skip.o").exists());
}

#[test]
fn extract_no_filter_flags_full_tree_regression() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"aa\n").unwrap();
    fs::write(src.join("sub").join("b.txt"), b"bb\n").unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("full.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = dir.path().join("out");
    let result = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        listing.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&result.stderr);
    assert!(
        err.contains("2 file"),
        "no filter ⇒ full tree (2 files); stderr={err}"
    );
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"aa\n");
    assert_eq!(fs::read(out.join("sub").join("b.txt")).unwrap(), b"bb\n");
}

#[test]
fn extract_illegal_exclude_errors_clearly() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let store = dir.path().join("store");
    let listing = dir.path().join("app.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let out = dir.path().join("out");
    let fail = run_fail(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--exclude",
        "a*b",
        listing.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("invalid exclude pattern") || err.contains("exclude pattern"),
        "stderr={err}"
    );
    assert!(!out.exists());
}

// --- Phase 13 M4: pull --path/--exclude ---

/// Like [`spawn_put_get_store_server`] but also counts GET requests (for subset pull proofs).
fn spawn_put_get_store_server_counting_gets(
    store_root: PathBuf,
    put_count: Arc<AtomicUsize>,
    get_count: Arc<AtomicUsize>,
) -> (String, thread::JoinHandle<()>) {
    let server = Server::http("127.0.0.1:0").expect("bind");
    let port = server.server_addr().to_ip().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let handle = thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let method = request.method().clone();
            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(&url);
            let rel = path.trim_start_matches('/');
            let file_path = store_root.join(rel);

            match method {
                Method::Head => {
                    if file_path.is_file() {
                        let len = fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
                        let response = Response::empty(200).with_header(
                            Header::from_bytes(&b"Content-Length"[..], len.to_string()).unwrap(),
                        );
                        let _ = request.respond(response);
                    } else {
                        let _ = request.respond(Response::empty(StatusCode(404)));
                    }
                }
                Method::Get => {
                    get_count.fetch_add(1, Ordering::SeqCst);
                    if file_path.is_file() {
                        let data = fs::read(&file_path).unwrap_or_default();
                        let _ = request.respond(Response::from_data(data));
                    } else {
                        let _ = request.respond(Response::empty(StatusCode(404)));
                    }
                }
                Method::Put | Method::Post => {
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                    put_count.fetch_add(1, Ordering::SeqCst);
                    if let Some(parent) = file_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(&file_path, &body);
                    let _ = request.respond(
                        Response::empty(StatusCode(200))
                            .with_header(Header::from_bytes(&b"Content-Length"[..], "0").unwrap()),
                    );
                }
                _ => {
                    let _ = request.respond(Response::empty(StatusCode(405)));
                }
            }
        }
    });
    thread::sleep(Duration::from_millis(20));
    (base, handle)
}

#[test]
fn pull_help_lists_path_exclude() {
    let help = run_ok(&["pull", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(s.contains("--path"), "pull --help should list --path:\n{s}");
    assert!(
        s.contains("--exclude"),
        "pull --help should list --exclude:\n{s}"
    );
}

#[test]
fn pull_path_subset_gets_only_filtered_chunks() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("packages").join("foo")).unwrap();
    fs::create_dir_all(src.join("packages").join("bar")).unwrap();
    fs::create_dir_all(src.join("other")).unwrap();
    // Distinct content ⇒ distinct chunk ids (small files stay single-chunk).
    fs::write(
        src.join("packages").join("foo").join("a.txt"),
        b"foo-a-unique\n",
    )
    .unwrap();
    fs::write(
        src.join("packages").join("bar").join("b.txt"),
        b"bar-b-unique\n",
    )
    .unwrap();
    fs::write(src.join("other").join("c.txt"), b"other-c-unique\n").unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("full.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let put_count = Arc::new(AtomicUsize::new(0));
    let get_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server_counting_gets(
        mirror.clone(),
        Arc::clone(&put_count),
        Arc::clone(&get_count),
    );

    run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base,
        listing.to_str().unwrap(),
    ]);
    assert!(
        put_count.load(Ordering::SeqCst) >= 3,
        "push should upload all file chunks; puts={}",
        put_count.load(Ordering::SeqCst)
    );

    // Full pull (no path filter) — baseline GET count.
    let full_store = dir.path().join("full_pull");
    get_count.store(0, Ordering::SeqCst);
    let full_json = run_ok(&[
        "pull",
        "--store",
        full_store.to_str().unwrap(),
        "--source",
        &base,
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let full_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&full_json.stdout).trim())
            .expect("full pull json");
    let full_unique = full_v["unique_chunks"].as_u64().unwrap();
    let full_gets = get_count.load(Ordering::SeqCst);
    assert!(
        full_unique >= 3,
        "full pull unique_chunks should cover 3 files; v={full_v}"
    );
    assert_eq!(
        full_gets as u64, full_unique,
        "empty store full pull: GET count should equal unique_chunks; gets={full_gets} v={full_v}"
    );

    // Subset pull --path packages/foo — fewer GETs / unique_chunks.
    let sub_store = dir.path().join("sub_pull");
    get_count.store(0, Ordering::SeqCst);
    let sub_json = run_ok(&[
        "pull",
        "--store",
        sub_store.to_str().unwrap(),
        "--source",
        &base,
        "--path",
        "packages/foo",
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let sub_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&sub_json.stdout).trim())
            .expect("subset pull json");
    let sub_unique = sub_v["unique_chunks"].as_u64().unwrap();
    let sub_gets = get_count.load(Ordering::SeqCst);
    assert!(
        sub_unique < full_unique,
        "subset unique_chunks must be < full; sub={sub_unique} full={full_unique} sub_v={sub_v}"
    );
    assert!(
        sub_unique >= 1,
        "subset should still fetch foo chunks; sub_v={sub_v}"
    );
    assert_eq!(
        sub_gets as u64, sub_unique,
        "empty store subset pull: GET count should equal filtered unique_chunks; gets={sub_gets} v={sub_v}"
    );
    assert!(
        sub_gets < full_gets,
        "subset GET count must be < full; sub={sub_gets} full={full_gets}"
    );
}

#[test]
fn pull_path_dry_run_json_unique_chunks_filtered() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("packages").join("foo")).unwrap();
    fs::create_dir_all(src.join("packages").join("bar")).unwrap();
    fs::write(src.join("packages").join("foo").join("a.txt"), b"foo-dry\n").unwrap();
    fs::write(src.join("packages").join("bar").join("b.txt"), b"bar-dry\n").unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("app.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let empty = dir.path().join("empty");
    let full = run_ok(&[
        "pull",
        "--store",
        empty.to_str().unwrap(),
        "--source",
        store.to_str().unwrap(),
        "--dry-run",
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let full_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&full.stdout).trim()).expect("full dry json");
    assert_eq!(full_v["dry_run"], true);
    let full_uc = full_v["unique_chunks"].as_u64().unwrap();
    assert!(full_uc >= 2, "full dry-run unique_chunks; v={full_v}");

    let sub = run_ok(&[
        "pull",
        "--store",
        empty.to_str().unwrap(),
        "--source",
        store.to_str().unwrap(),
        "--path",
        "packages/foo",
        "--dry-run",
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let sub_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&sub.stdout).trim()).expect("sub dry json");
    assert_eq!(sub_v["dry_run"], true);
    let sub_uc = sub_v["unique_chunks"].as_u64().unwrap();
    assert!(
        sub_uc < full_uc && sub_uc >= 1,
        "filtered unique_chunks; full={full_uc} sub={sub_uc} sub_v={sub_v}"
    );
    // dry-run must not create store
    assert!(!empty.join("meta.toml").is_file());
}

#[test]
fn pull_exclude_filters_junk_unique_chunks() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("pkg")).unwrap();
    fs::create_dir_all(src.join("junk")).unwrap();
    fs::write(src.join("pkg").join("keep.txt"), b"keep-me\n").unwrap();
    fs::write(src.join("junk").join("noise.txt"), b"noise-data\n").unwrap();
    fs::write(src.join("skip.o"), b"object\n").unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("app.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let empty = dir.path().join("empty");
    let full = run_ok(&[
        "pull",
        "--store",
        empty.to_str().unwrap(),
        "--source",
        store.to_str().unwrap(),
        "--dry-run",
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let full_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&full.stdout).trim()).unwrap();
    let full_uc = full_v["unique_chunks"].as_u64().unwrap();

    let filtered = run_ok(&[
        "pull",
        "--store",
        empty.to_str().unwrap(),
        "--source",
        store.to_str().unwrap(),
        "--exclude",
        "junk/",
        "--exclude",
        "*.o",
        "--dry-run",
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let fv: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&filtered.stdout).trim()).unwrap();
    let filtered_uc = fv["unique_chunks"].as_u64().unwrap();
    assert!(
        filtered_uc < full_uc && filtered_uc >= 1,
        "exclude should shrink unique_chunks; full={full_uc} filtered={filtered_uc} fv={fv}"
    );
}

#[test]
fn pull_no_filter_flags_full_set_regression() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"aa-pull\n").unwrap();
    fs::write(src.join("sub").join("b.txt"), b"bb-pull\n").unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("full.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let newstore = dir.path().join("newstore");
    let out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        store.to_str().unwrap(),
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(v["ok"], true);
    assert!(
        v["unique_chunks"].as_u64().unwrap() >= 2,
        "no filter ⇒ full set; v={v}"
    );
    assert!(
        v["fetched"].as_u64().unwrap() >= 2 || v["skipped"].as_u64().unwrap() >= 2,
        "should fetch or skip all; v={v}"
    );
    assert!(count_cnk(&newstore.join("chunks")) >= 2);
}

#[test]
fn pull_illegal_exclude_errors_clearly() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let store = dir.path().join("store");
    let listing = dir.path().join("app.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let empty = dir.path().join("empty");
    let fail = run_fail(&[
        "pull",
        "--store",
        empty.to_str().unwrap(),
        "--source",
        store.to_str().unwrap(),
        "--exclude",
        "a*b",
        listing.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("invalid exclude pattern") || err.contains("exclude pattern"),
        "stderr={err}"
    );
}

#[test]
fn push_help_lists_path_exclude() {
    let p = run_ok(&["push", "--help"]);
    let s = String::from_utf8_lossy(&p.stdout);
    assert!(s.contains("--path"), "push --help should list --path:\n{s}");
    assert!(
        s.contains("--exclude"),
        "push --help should list --exclude:\n{s}"
    );
}

#[test]
fn push_path_subset_puts_only_filtered_chunks() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("packages").join("foo")).unwrap();
    fs::create_dir_all(src.join("packages").join("bar")).unwrap();
    fs::create_dir_all(src.join("other")).unwrap();
    fs::write(
        src.join("packages").join("foo").join("a.txt"),
        b"foo-a-unique-push\n",
    )
    .unwrap();
    fs::write(
        src.join("packages").join("bar").join("b.txt"),
        b"bar-b-unique-push\n",
    )
    .unwrap();
    fs::write(src.join("other").join("c.txt"), b"other-c-unique-push\n").unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("full.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    // Full push — baseline PUT count.
    let mirror_full = dir.path().join("mirror_full");
    fs::create_dir_all(&mirror_full).unwrap();
    let put_full = Arc::new(AtomicUsize::new(0));
    let (base_full, _h1) = spawn_put_get_store_server(mirror_full.clone(), Arc::clone(&put_full));
    let full_json = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base_full,
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let full_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&full_json.stdout).trim())
            .expect("full push json");
    let full_unique = full_v["unique_chunks"].as_u64().unwrap();
    let full_puts = put_full.load(Ordering::SeqCst);
    assert!(
        full_unique >= 3,
        "full push unique_chunks should cover 3 files; v={full_v}"
    );
    assert_eq!(
        full_puts as u64, full_unique,
        "empty dest full push: PUT count should equal unique_chunks; puts={full_puts} v={full_v}"
    );

    // Subset push --path packages/foo — fewer PUTs / unique_chunks.
    let mirror_sub = dir.path().join("mirror_sub");
    fs::create_dir_all(&mirror_sub).unwrap();
    let put_sub = Arc::new(AtomicUsize::new(0));
    let (base_sub, _h2) = spawn_put_get_store_server(mirror_sub.clone(), Arc::clone(&put_sub));
    let sub_json = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base_sub,
        "--path",
        "packages/foo",
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let sub_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&sub_json.stdout).trim())
            .expect("subset push json");
    let sub_unique = sub_v["unique_chunks"].as_u64().unwrap();
    let sub_puts = put_sub.load(Ordering::SeqCst);
    assert!(
        sub_unique < full_unique,
        "subset unique_chunks must be < full; sub={sub_unique} full={full_unique} sub_v={sub_v}"
    );
    assert!(
        sub_unique >= 1,
        "subset should still upload foo chunks; sub_v={sub_v}"
    );
    assert_eq!(
        sub_puts as u64, sub_unique,
        "empty dest subset push: PUT count should equal filtered unique_chunks; puts={sub_puts} v={sub_v}"
    );
    assert!(
        sub_puts < full_puts,
        "subset PUT count must be < full; sub={sub_puts} full={full_puts}"
    );
}

#[test]
fn push_cfidx_with_path_errors_clearly() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));

    let fail = run_fail(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base,
        "--path",
        "anything",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("--path/--exclude")
            || err.contains("looks like a `.cfidx`")
            || err.contains("looks like a .cfidx")
            || err.contains("cfidx"),
        "stderr should explain cfidx+path is invalid; stderr={err}"
    );
    assert_eq!(
        put_count.load(Ordering::SeqCst),
        0,
        "must not silently upload on cfidx+path"
    );
}

#[test]
fn push_cfidx_with_exclude_errors_clearly() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let fail = run_fail(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        "http://127.0.0.1:9",
        "--exclude",
        "*.o",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("--path/--exclude")
            || err.contains("looks like a `.cfidx`")
            || err.contains("cfidx"),
        "stderr should explain cfidx+exclude is invalid; stderr={err}"
    );
}

#[test]
fn push_no_filter_flags_full_set_regression() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"aa-push\n").unwrap();
    fs::write(src.join("sub").join("b.txt"), b"bb-push\n").unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("full.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));
    let out = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base,
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(v["ok"], true);
    assert!(
        v["unique_chunks"].as_u64().unwrap() >= 2,
        "no filter ⇒ full set; v={v}"
    );
    assert!(
        put_count.load(Ordering::SeqCst) as u64 >= 2,
        "should upload all; puts={} v={v}",
        put_count.load(Ordering::SeqCst)
    );
}

#[test]
fn push_path_dry_run_json_unique_chunks_filtered() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("packages").join("foo")).unwrap();
    fs::create_dir_all(src.join("packages").join("bar")).unwrap();
    fs::write(
        src.join("packages").join("foo").join("a.txt"),
        b"foo-dry-p\n",
    )
    .unwrap();
    fs::write(
        src.join("packages").join("bar").join("b.txt"),
        b"bar-dry-p\n",
    )
    .unwrap();

    let store = dir.path().join("store");
    let listing = dir.path().join("app.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));

    let full = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base,
        "--dry-run",
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let full_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&full.stdout).trim()).expect("full dry json");
    assert_eq!(full_v["dry_run"], true);
    let full_uc = full_v["unique_chunks"].as_u64().unwrap();
    assert!(full_uc >= 2, "full dry-run unique_chunks; v={full_v}");

    let sub = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base,
        "--path",
        "packages/foo",
        "--dry-run",
        "--format",
        "json",
        listing.to_str().unwrap(),
    ]);
    let sub_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&sub.stdout).trim()).expect("sub dry json");
    assert_eq!(sub_v["dry_run"], true);
    let sub_uc = sub_v["unique_chunks"].as_u64().unwrap();
    assert!(
        sub_uc < full_uc && sub_uc >= 1,
        "filtered unique_chunks; full={full_uc} sub={sub_uc} sub_v={sub_v}"
    );
    assert_eq!(
        put_count.load(Ordering::SeqCst),
        0,
        "dry-run must issue no PUT"
    );
}

#[test]
fn push_illegal_exclude_errors_clearly() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let store = dir.path().join("store");
    let listing = dir.path().join("app.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let fail = run_fail(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        "http://127.0.0.1:9",
        "--exclude",
        "a*b",
        listing.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("invalid exclude pattern") || err.contains("exclude pattern"),
        "stderr={err}"
    );
}

#[test]
fn push_cfidx_without_path_flags_unchanged() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let mirror = dir.path().join("mirror");
    fs::create_dir_all(&mirror).unwrap();
    let put_count = Arc::new(AtomicUsize::new(0));
    let (base, _handle) = spawn_put_get_store_server(mirror, Arc::clone(&put_count));
    let out = run_ok(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        &base,
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(v["ok"], true);
    assert!(
        v["unique_chunks"].as_u64().unwrap() >= 1,
        "cfidx without path flags ⇒ full blob set; v={v}"
    );
    assert!(put_count.load(Ordering::SeqCst) >= 1);
}

// --- Phase 14 M4: --exclude-from ---

#[test]
fn exclude_from_help_on_archive_extract_pull_push() {
    for cmd in ["archive", "extract", "pull", "push"] {
        let help = run_ok(&[cmd, "--help"]);
        let s = String::from_utf8_lossy(&help.stdout);
        assert!(
            s.contains("--exclude-from"),
            "{cmd} --help should list --exclude-from:\n{s}"
        );
    }
}

#[test]
fn archive_exclude_from_filters_and_merges_with_cli_exclude() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("junk")).unwrap();
    fs::write(src.join("a.txt"), b"keep\n").unwrap();
    fs::write(src.join("skip.o"), b"obj\n").unwrap();
    fs::write(src.join("junk").join("noise.txt"), b"noise\n").unwrap();
    fs::write(src.join("secret.txt"), b"nope\n").unwrap();

    let store = dir.path().join("store");
    let full = dir.path().join("full.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        full.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let bytes = fs::read(&full).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).unwrap();
    let paths: Vec<_> = arch.entries.iter().map(|e| e.path.as_str()).collect();
    assert!(
        paths.contains(&"skip.o"),
        "no flags ≡ full tree; paths={paths:?}"
    );
    assert!(paths.contains(&"junk/noise.txt"), "paths={paths:?}");
    assert!(paths.contains(&"secret.txt"), "paths={paths:?}");

    let ex1 = dir.path().join("ex1.txt");
    let ex2 = dir.path().join("ex2.txt");
    fs::write(&ex1, "# objs\n\n  *.o  \n").unwrap();
    fs::write(&ex2, "secret.txt\n").unwrap();

    let filtered = dir.path().join("filt.cfdir");
    let result = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        filtered.to_str().unwrap(),
        "--exclude-from",
        ex1.to_str().unwrap(),
        "--exclude-from",
        ex2.to_str().unwrap(),
        "--exclude",
        "junk/",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&result.stderr);
    assert!(
        err.contains("excluded=") && !err.contains("excluded=0"),
        "stderr={err}"
    );
    let bytes = fs::read(&filtered).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).unwrap();
    let paths: Vec<_> = arch
        .entries
        .iter()
        .map(|e| e.path.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec!["a.txt".to_string()],
        "merged filter paths={paths:?}"
    );
}

#[test]
fn archive_exclude_from_illegal_line_and_missing_file() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let store = dir.path().join("store");
    let out = dir.path().join("app.cfdir");
    let bad = dir.path().join("bad.txt");
    fs::write(&bad, "# ok\na*b\n").unwrap();

    let fail = run_fail(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--exclude-from",
        bad.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("invalid exclude pattern"),
        "illegal line should share --exclude error class; stderr={err}"
    );

    let missing = dir.path().join("no-such-excludes.txt");
    let fail = run_fail(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--exclude-from",
        missing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("exclude file") || err.contains("cannot read"),
        "missing file should be a clear error; stderr={err}"
    );
}

#[test]
fn extract_pull_push_exclude_from_missing_file_nonzero() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("missing-excludes.txt");
    let listing = dir.path().join("nope.cfdir");
    let store = dir.path().join("store");

    for args in [
        vec![
            "extract".to_string(),
            "--store".into(),
            store.display().to_string(),
            "-o".into(),
            dir.path().join("out").display().to_string(),
            "--exclude-from".into(),
            missing.display().to_string(),
            listing.display().to_string(),
        ],
        vec![
            "pull".into(),
            "--store".into(),
            store.display().to_string(),
            "--source".into(),
            store.display().to_string(),
            "--exclude-from".into(),
            missing.display().to_string(),
            listing.display().to_string(),
        ],
        vec![
            "push".into(),
            "--store".into(),
            store.display().to_string(),
            "--dest".into(),
            "http://127.0.0.1:9".into(),
            "--exclude-from".into(),
            missing.display().to_string(),
            listing.display().to_string(),
        ],
    ] {
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let fail = run_fail(&refs);
        let err = String::from_utf8_lossy(&fail.stderr);
        assert!(
            err.contains("exclude file") || err.contains("cannot read"),
            "args={refs:?} stderr={err}"
        );
    }
}

// --- Phase 15 M2: CLI --cache-max-bytes ---

fn cache_stats_bytes(store: &Path) -> u64 {
    let out = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stats json");
    v["bytes_on_disk"].as_u64().expect("bytes_on_disk")
}

#[test]
fn help_lists_cache_max_bytes_on_cat_verify_extract_mount() {
    for cmd in ["cat", "verify", "extract", "mount"] {
        let help = run_ok(&[cmd, "--help"]);
        let s = String::from_utf8_lossy(&help.stdout);
        assert!(
            s.contains("--cache-max-bytes"),
            "{cmd} --help must list --cache-max-bytes:\n{s}"
        );
        assert!(
            s.contains("--cache"),
            "{cmd} --help must still list --cache:\n{s}"
        );
        // Phase16-M3: value_name SIZE + human suffix hint in help.
        assert!(
            s.contains("SIZE") || s.contains("<SIZE>"),
            "{cmd} --help must use SIZE value_name for --cache-max-bytes:\n{s}"
        );
        assert!(
            s.contains("K/M/G") || (s.contains("Ki") && s.contains("Mi")),
            "{cmd} --help must mention K/M/G/Ki/Mi/Gi suffixes:\n{s}"
        );
    }
}

// --- Phase 16 M3: human byte suffixes on --cache-max-bytes ---

#[test]
fn cache_max_bytes_accepts_human_suffix_1m() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cache = dir.path().join("cache");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("out.bin");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "--cache-max-bytes",
        "1M",
        idx.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out).unwrap());
    // hello.txt is tiny; 1M budget easily fits → cache should have filled.
    assert!(
        cache_stats_bytes(&cache) > 0,
        "1M budget should allow fill of tiny hello.txt"
    );
}

#[test]
fn cache_max_bytes_rejects_decimal_and_b_suffix() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cache = dir.path().join("cache");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("out.bin");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    for bad in ["1.5M", "64MB", "1KB"] {
        let fail = run_fail(&[
            "cat",
            "--store",
            store.to_str().unwrap(),
            "--cache",
            cache.to_str().unwrap(),
            "--cache-max-bytes",
            bad,
            idx.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ]);
        let err = format!(
            "{}{}",
            String::from_utf8_lossy(&fail.stderr),
            String::from_utf8_lossy(&fail.stdout)
        );
        assert!(
            !err.is_empty(),
            "expected clear error for --cache-max-bytes {bad}"
        );
    }
}

#[test]
fn cache_max_bytes_without_cache_errors_on_four_commands() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("out.bin");
    let input = fixtures_dir().join("hello.txt");
    let archive = dir.path().join("tree.cfdir");
    let extract_out = dir.path().join("extracted");
    let mnt = dir.path().join("mnt");
    fs::create_dir_all(&mnt).unwrap();

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    // Minimal .cfdir for extract/mount error path (origin opens before materialize).
    let src_tree = dir.path().join("src");
    fs::create_dir_all(src_tree.join("a")).unwrap();
    fs::write(src_tree.join("a/f.txt"), b"x").unwrap();
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
        src_tree.to_str().unwrap(),
    ]);

    let cases: Vec<Vec<&str>> = vec![
        vec![
            "cat",
            "--store",
            store.to_str().unwrap(),
            "--cache-max-bytes",
            "1024",
            idx.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ],
        vec![
            "verify",
            "--store",
            store.to_str().unwrap(),
            "--cache-max-bytes",
            "1024",
            idx.to_str().unwrap(),
        ],
        vec![
            "extract",
            "--store",
            store.to_str().unwrap(),
            "--cache-max-bytes",
            "1024",
            "-o",
            extract_out.to_str().unwrap(),
            archive.to_str().unwrap(),
        ],
        vec![
            "mount",
            "--store",
            store.to_str().unwrap(),
            "--cache-max-bytes",
            "1024",
            idx.to_str().unwrap(),
            mnt.to_str().unwrap(),
        ],
    ];

    for args in cases {
        let fail = run_fail(&args);
        let err = String::from_utf8_lossy(&fail.stderr);
        assert!(
            err.contains("--cache-max-bytes") && err.contains("--cache"),
            "args={args:?} stderr={err}"
        );
    }
}

#[test]
fn cat_verify_cache_max_bytes_caps_disk_and_still_serves() {
    let dir = tempdir().unwrap();
    let primary = dir.path().join("primary");
    let cache_budget = dir.path().join("cache-budget");
    let cache_unbounded = dir.path().join("cache-unbounded");
    let cache_zero = dir.path().join("cache-zero");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("reassembled");
    let input = dir.path().join("multi.bin");
    // Patterned 64 KiB → many distinct chunks under small FastCDC params.
    let mut bytes = Vec::with_capacity(64 * 1024);
    for i in 0..(64 * 1024) {
        bytes.push(((i * 17) ^ (i >> 3)) as u8);
    }
    fs::write(&input, &bytes).unwrap();

    run_ok(&[
        "make",
        "--store",
        primary.to_str().unwrap(),
        "--chunk-size",
        "2048:4096:8192",
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // Unbounded --cache fills (baseline).
    run_ok(&[
        "verify",
        "--store",
        primary.to_str().unwrap(),
        "--cache",
        cache_unbounded.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let unbounded = cache_stats_bytes(&cache_unbounded);
    assert!(
        unbounded > 0,
        "unbounded cache should fill, got bytes_on_disk={unbounded}"
    );

    // Soft budget smaller than full fill: disk must not exceed N.
    let max = unbounded / 2;
    assert!(
        max > 0,
        "need positive half-budget from unbounded={unbounded}"
    );
    let max_s = max.to_string();
    run_ok(&[
        "verify",
        "--store",
        primary.to_str().unwrap(),
        "--cache",
        cache_budget.to_str().unwrap(),
        "--cache-max-bytes",
        &max_s,
        idx.to_str().unwrap(),
    ]);
    let capped = cache_stats_bytes(&cache_budget);
    assert!(
        capped <= max,
        "budgeted cache bytes_on_disk={capped} must be <= max={max} (unbounded={unbounded})"
    );
    assert!(
        capped < unbounded,
        "budgeted cache ({capped}) should be strictly under unbounded ({unbounded})"
    );

    // max=0 → refuse all fills; cat still reassembles from primary.
    run_ok(&[
        "cat",
        "--store",
        primary.to_str().unwrap(),
        "--cache",
        cache_zero.to_str().unwrap(),
        "--cache-max-bytes",
        "0",
        idx.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(cache_stats_bytes(&cache_zero), 0);
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out).unwrap());
}

#[test]
fn extract_cache_max_bytes_caps_disk() {
    let dir = tempdir().unwrap();
    let primary = dir.path().join("primary");
    let cache = dir.path().join("cache");
    let archive = dir.path().join("tree.cfdir");
    let extract_out = dir.path().join("out");
    let src_tree = dir.path().join("src");
    fs::create_dir_all(&src_tree).unwrap();
    let mut bytes = Vec::with_capacity(64 * 1024);
    for i in 0..(64 * 1024) {
        bytes.push(((i * 31) ^ 0x5a) as u8);
    }
    fs::write(src_tree.join("blob.bin"), &bytes).unwrap();

    run_ok(&[
        "archive",
        "--store",
        primary.to_str().unwrap(),
        "--chunk-size",
        "2048:4096:8192",
        "-o",
        archive.to_str().unwrap(),
        src_tree.to_str().unwrap(),
    ]);

    // First: unbounded fill to learn size.
    let cache_full = dir.path().join("cache-full");
    run_ok(&[
        "extract",
        "--store",
        primary.to_str().unwrap(),
        "--cache",
        cache_full.to_str().unwrap(),
        "-o",
        dir.path().join("out-full").to_str().unwrap(),
        archive.to_str().unwrap(),
    ]);
    let unbounded = cache_stats_bytes(&cache_full);
    assert!(unbounded > 0, "unbounded extract cache fill expected");

    let max = unbounded / 2;
    let max_s = max.to_string();
    run_ok(&[
        "extract",
        "--store",
        primary.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "--cache-max-bytes",
        &max_s,
        "-o",
        extract_out.to_str().unwrap(),
        archive.to_str().unwrap(),
    ]);
    let capped = cache_stats_bytes(&cache);
    assert!(
        capped <= max,
        "extract budgeted cache bytes_on_disk={capped} must be <= max={max}"
    );
    // Extracted content still correct (served from primary when fill refused).
    assert_eq!(
        fs::read(src_tree.join("blob.bin")).unwrap(),
        fs::read(extract_out.join("blob.bin")).unwrap()
    );
}

// --- Phase 15 M3: make --format text|json ---

#[test]
fn make_help_lists_format() {
    let out = run_ok(&["make", "--help"]);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("--format"),
        "make --help must list --format:\n{s}"
    );
    assert!(
        s.contains("text") && s.contains("json"),
        "make --help --format should mention text|json:\n{s}"
    );
}

#[test]
fn make_default_format_is_text_not_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    let out = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("make: wrote"),
        "default text must keep stderr summary; stderr={stderr}"
    );
    let (new_n, reused_n) = parse_make_stats(&stderr);
    assert!(new_n >= 1, "expected new chunks; new={new_n}");
    assert_eq!(reused_n, 0, "first make should reuse=0");
    assert!(
        !stdout.trim().starts_with('{'),
        "default (no --format) stdout must not be pure JSON; got {stdout}"
    );
    assert!(idx.is_file(), "index must be written");
}

#[test]
fn make_format_text_explicit_matches_default() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    let out = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--format",
        "text",
        input.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("make: wrote") && stderr.contains("new="),
        "explicit --format text must keep stderr summary; stderr={stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.trim().starts_with('{'),
        "text format must not emit JSON on stdout; got {stdout}"
    );
}

#[test]
fn make_format_json_parses_and_has_fields() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");
    let input_bytes = fs::metadata(&input).unwrap().len();

    let out = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--format",
        "json",
        input.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("make: wrote"),
        "json must not dual-write text summary; stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("make json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["bytes"].as_u64(), Some(input_bytes));
    let chunks = v["chunks"].as_u64().expect("chunks field");
    assert!(chunks >= 1, "chunks={chunks}");
    let new_n = v["new"].as_u64().expect("new field");
    let reused = v["reused"].as_u64().expect("reused field");
    assert_eq!(new_n + reused, chunks, "new+reused must equal chunks");
    assert!(idx.is_file(), "index must still be written under json");
}

#[test]
fn make_format_json_reused_on_second_make() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx1 = dir.path().join("a.cfidx");
    let idx2 = dir.path().join("b.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx1.to_str().unwrap(),
        "--format",
        "json",
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx2.to_str().unwrap(),
        "--format",
        "json",
        input.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("make json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    let chunks = v["chunks"].as_u64().expect("chunks");
    assert_eq!(
        v["new"].as_u64(),
        Some(0),
        "second make should write no new chunks"
    );
    assert_eq!(
        v["reused"].as_u64(),
        Some(chunks),
        "second make should reuse all chunks"
    );
}

// --- Phase 15 M4: cat --format text|json ---

#[test]
fn cat_help_lists_format() {
    let out = run_ok(&["cat", "--help"]);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("--format"),
        "cat --help must list --format:\n{s}"
    );
    assert!(
        s.contains("text") && s.contains("json"),
        "cat --help --format should mention text|json:\n{s}"
    );
}

#[test]
fn cat_default_format_is_text_not_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let out_path = dir.path().join("reassembled");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
        "-o",
        out_path.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.trim().starts_with('{'),
        "default (no --format) stdout must not be pure JSON; got {stdout}"
    );
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out_path).unwrap());
}

#[test]
fn cat_format_text_explicit_matches_default() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let out_path = dir.path().join("reassembled");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "text",
        idx.to_str().unwrap(),
        "-o",
        out_path.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.trim().starts_with('{'),
        "text format must not emit JSON on stdout; got {stdout}"
    );
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out_path).unwrap());
}

#[test]
fn cat_format_json_parses_and_has_fields() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let out_path = dir.path().join("reassembled");
    let input = fixtures_dir().join("hello.txt");
    let input_bytes = fs::metadata(&input).unwrap().len();

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        idx.to_str().unwrap(),
        "-o",
        out_path.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("\"ok\""),
        "json must not dual-write JSON on stderr; stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("cat json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["bytes"].as_u64(), Some(input_bytes));
    assert!(
        v.get("cache_hits").is_none()
            && v.get("cache_miss_fills").is_none()
            && v.get("cache_miss_refused").is_none(),
        "without --cache, cache_* keys must be omitted; got {v}"
    );
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out_path).unwrap());
    assert_eq!(
        fs::metadata(&out_path).unwrap().len(),
        input_bytes,
        "-o payload size must match bytes field"
    );
}

#[test]
fn cat_format_json_with_cache_max_bytes_smoke() {
    let dir = tempdir().unwrap();
    let primary = dir.path().join("primary");
    let cache = dir.path().join("cache");
    let idx = dir.path().join("out.cfidx");
    let out_path = dir.path().join("reassembled");
    let input = fixtures_dir().join("hello.txt");
    let input_bytes = fs::metadata(&input).unwrap().len();

    run_ok(&[
        "make",
        "--store",
        primary.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "cat",
        "--source",
        primary.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "--cache-max-bytes",
        "1",
        "--format",
        "json",
        idx.to_str().unwrap(),
        "-o",
        out_path.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("cat json+cache-max invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["bytes"].as_u64(), Some(input_bytes));
    assert!(
        v.get("cache_hits").is_some()
            && v.get("cache_miss_fills").is_some()
            && v.get("cache_miss_refused").is_some(),
        "--cache + --format json must add cache_* fields; got {v}"
    );
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out_path).unwrap());
}

// --- Phase16-M2: repeatable `--fallback` on read commands ---

#[test]
fn fallback_help_on_read_commands_not_on_push() {
    for cmd in ["cat", "pull", "verify", "extract", "mount", "doctor"] {
        let out = Command::new(bin())
            .args([cmd, "--help"])
            .output()
            .expect("spawn");
        assert!(out.status.success(), "{cmd} --help failed");
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(
            s.contains("--fallback"),
            "{cmd} --help missing --fallback:\n{s}"
        );
    }
    let push = Command::new(bin())
        .args(["push", "--help"])
        .output()
        .expect("spawn");
    assert!(push.status.success());
    let s = String::from_utf8_lossy(&push.stdout);
    assert!(
        !s.contains("--fallback"),
        "push must not expose --fallback:\n{s}"
    );
}

#[test]
fn fallback_cat_primary_miss_fallback_hit() {
    let dir = tempdir().unwrap();
    let primary = dir.path().join("primary");
    let fallback = dir.path().join("fallback");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("reassembled");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        fallback.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // Empty primary store (meta only) — chunks live only in fallback.
    fs::create_dir_all(&primary).unwrap();
    fs::write(
        primary.join("meta.toml"),
        "# ChunkForge local CAS store metadata\n\
         magic = \"CFSTORE\"\n\
         version = 1\n\
         compression = \"none\"\n",
    )
    .unwrap();

    // Without --fallback: missing chunks → fail.
    run_fail(&[
        "cat",
        "--store",
        primary.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);

    // With --fallback: Missing on primary, hit on fallback → success.
    run_ok(&[
        "cat",
        "--store",
        primary.to_str().unwrap(),
        "--fallback",
        fallback.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out).unwrap());

    // verify / doctor also accept the chain.
    run_ok(&[
        "verify",
        "--store",
        primary.to_str().unwrap(),
        "--fallback",
        fallback.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    run_ok(&[
        "doctor",
        "--store",
        primary.to_str().unwrap(),
        "--fallback",
        fallback.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
}

#[test]
fn fallback_zero_times_equiv_single_origin() {
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
    run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out).unwrap());
}

#[test]
fn fallback_local_primary_http_fallback_with_url_template() {
    // Local primary + HTTP fallback + --url-template must not bail (templates
    // apply to the HTTP fallback; no-op on local primary).
    let dir = tempdir().unwrap();
    let primary = dir.path().join("primary");
    let mirror = dir.path().join("mirror");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("reassembled");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        mirror.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    fs::create_dir_all(&primary).unwrap();
    fs::write(
        primary.join("meta.toml"),
        "# ChunkForge local CAS store metadata\n\
         magic = \"CFSTORE\"\n\
         version = 1\n\
         compression = \"none\"\n",
    )
    .unwrap();

    let (base, _handle) = spawn_static_store_server(mirror);

    run_ok(&[
        "cat",
        "--store",
        primary.to_str().unwrap(),
        "--fallback",
        &base,
        "--url-template",
        "{base}/{path}",
        "-o",
        out.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out).unwrap());
}

// --- Phase 17 M2: make/archive --compression ---

#[test]
fn help_lists_compression_on_make_and_archive() {
    for cmd in ["make", "archive"] {
        let out = Command::new(bin())
            .args([cmd, "--help"])
            .output()
            .expect("spawn");
        assert!(out.status.success(), "{cmd} --help failed");
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(
            s.contains("--compression"),
            "{cmd} --help must list --compression:\n{s}"
        );
        let lower = s.to_ascii_lowercase();
        assert!(
            lower.contains("none") && lower.contains("zstd"),
            "{cmd} --help should mention none|zstd:\n{s}"
        );
        assert!(
            lower.contains("new") || lower.contains("creat"),
            "{cmd} --help should say flag affects new stores:\n{s}"
        );
        assert!(
            !lower.contains("content-encoding")
                || lower.contains("not")
                || lower.contains("≠")
                || lower.contains("wire")
                || lower.contains("http"),
            "{cmd} --help should clarify disk ≠ HTTP wire (got):\n{s}"
        );
    }
}

#[test]
fn make_compression_zstd_stats_and_cat_roundtrip() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store-z");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("reassembled");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "--compression",
        "zstd",
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let stats = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&stats.stdout).trim()).expect("stats json");
    assert_eq!(v["compression"].as_str(), Some("zstd"), "{v}");
    assert!(v["ok"].as_bool().unwrap_or(false), "{v}");

    // Optional plaintext round-trip via cat (get returns plaintext).
    run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out).unwrap());
}

#[test]
fn make_omitted_compression_is_none() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store-none");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let stats = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&stats.stdout).trim()).expect("stats json");
    assert_eq!(v["compression"].as_str(), Some("none"), "{v}");
}

#[test]
fn make_existing_none_store_omit_succeeds() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx1 = dir.path().join("a.cfidx");
    let idx2 = dir.path().join("b.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx1.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    // Omit again on existing none store → success (1.6 regression).
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx2.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let stats = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&stats.stdout).trim()).expect("stats json");
    assert_eq!(v["compression"].as_str(), Some("none"), "{v}");
}

#[test]
fn make_existing_none_store_explicit_zstd_mismatch() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx1 = dir.path().join("a.cfidx");
    let idx2 = dir.path().join("b.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx1.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let fail = run_fail(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "--compression",
        "zstd",
        "-o",
        idx2.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.to_ascii_lowercase().contains("compression mismatch")
            || err.to_ascii_lowercase().contains("mismatch"),
        "expected mismatch error, got:\n{err}"
    );
}

#[test]
fn make_compression_invalid_value_errors() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");
    let fail = run_fail(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "--compression",
        "gzip",
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&fail.stderr),
        String::from_utf8_lossy(&fail.stdout)
    );
    let lower = err.to_ascii_lowercase();
    assert!(
        lower.contains("compression") || lower.contains("gzip") || lower.contains("none|zstd"),
        "expected clear invalid-value error, got:\n{err}"
    );
}

#[test]
fn make_compression_case_insensitive_zstd() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "--compression",
        "Zstd",
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let stats = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&stats.stdout).trim()).expect("stats json");
    assert_eq!(v["compression"].as_str(), Some("zstd"), "{v}");
}

#[test]
fn archive_compression_zstd_stats() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store-z");
    let cfdir = dir.path().join("tree.cfdir");
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"archive zstd payload aaa").unwrap();

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "--compression",
        "zstd",
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let stats = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&stats.stdout).trim()).expect("stats json");
    assert_eq!(v["compression"].as_str(), Some("zstd"), "{v}");
}

#[test]
fn archive_omitted_compression_is_none() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"archive none payload").unwrap();

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let stats = run_ok(&[
        "store",
        "stats",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&stats.stdout).trim()).expect("stats json");
    assert_eq!(v["compression"].as_str(), Some("none"), "{v}");
}

// --- Phase 17 M3: archive --progress ---

#[test]
fn archive_help_lists_progress() {
    let help = run_ok(&["archive", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--progress"),
        "archive --help must list --progress:\n{s}"
    );
}

#[test]
fn archive_progress_emits_stderr_and_default_silent() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"progress file a").unwrap();
    fs::write(src.join("sub/b.txt"), b"progress file b").unwrap();
    fs::write(src.join("c.txt"), b"progress file c").unwrap();

    let with = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        "--progress",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&with.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=archive done=")),
        "archive --progress stderr must contain progress: op=archive; stderr={err}"
    );
    assert!(
        err.contains("done=3/3") || err.lines().any(|l| l.contains("done=") && l.contains("/3")),
        "archive progress TOTAL should be filtered file count (3); stderr={err}"
    );
    assert!(
        err.contains("archive: wrote"),
        "text summary still on stderr; stderr={err}"
    );

    let store2 = dir.path().join("store2");
    let cfdir2 = dir.path().join("tree2.cfdir");
    let without = run_ok(&[
        "archive",
        "--store",
        store2.to_str().unwrap(),
        "-o",
        cfdir2.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err0 = String::from_utf8_lossy(&without.stderr);
    assert!(
        !err0.contains("progress:"),
        "archive without --progress must not emit progress:; stderr={err0}"
    );
}

#[test]
fn archive_progress_orthogonal_to_format_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"json progress a").unwrap();
    fs::write(src.join("b.txt"), b"json progress b").unwrap();

    let out = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        "--progress",
        "--format",
        "json",
        src.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("archive json");
    assert_eq!(v["ok"].as_bool(), Some(true), "{v}");
    assert!(
        !stdout.contains("progress:"),
        "progress must not land on stdout with --format json; stdout={stdout}"
    );
    assert!(
        stderr
            .lines()
            .any(|l| l.starts_with("progress: op=archive done=")),
        "progress on stderr with json; stderr={stderr}"
    );
}

#[test]
fn archive_progress_counts_filtered_files_only() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("keep")).unwrap();
    fs::create_dir_all(src.join("skip")).unwrap();
    fs::write(src.join("keep/a.txt"), b"keep a").unwrap();
    fs::write(src.join("keep/b.txt"), b"keep b").unwrap();
    fs::write(src.join("skip/c.txt"), b"skip c").unwrap();

    let out = run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        "--progress",
        "--path",
        "keep",
        "--dry-run",
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=archive done=")),
        "dry-run --progress should still tick; stderr={err}"
    );
    assert!(
        err.contains("/2"),
        "TOTAL should be PathFilter-kept files (2), not 3; stderr={err}"
    );
    assert!(
        !err.contains("/3"),
        "excluded file must not inflate TOTAL; stderr={err}"
    );
}

// --- Phase 17 M4: extract / make --progress ---

#[test]
fn extract_and_make_help_list_progress() {
    for cmd in ["extract", "make"] {
        let help = run_ok(&[cmd, "--help"]);
        let s = String::from_utf8_lossy(&help.stdout);
        assert!(
            s.contains("--progress"),
            "{cmd} --help must list --progress:\n{s}"
        );
    }
}

#[test]
fn make_progress_emits_stderr_and_default_silent() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("blob.cfidx");
    let input = dir.path().join("in.bin");
    fs::write(&input, b"make progress payload bytes").unwrap();

    let with = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--progress",
        input.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&with.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=make done=")),
        "make --progress stderr must contain progress: op=make; stderr={err}"
    );
    // Granularity: single input file → TOTAL=1
    assert!(
        err.contains("done=1/1"),
        "make progress should be 1/1 (single-file unit); stderr={err}"
    );
    assert!(
        err.contains("make: wrote"),
        "text summary still on stderr; stderr={err}"
    );

    let store2 = dir.path().join("store2");
    let idx2 = dir.path().join("blob2.cfidx");
    let without = run_ok(&[
        "make",
        "--store",
        store2.to_str().unwrap(),
        "-o",
        idx2.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let err0 = String::from_utf8_lossy(&without.stderr);
    assert!(
        !err0.contains("progress:"),
        "make without --progress must not emit progress:; stderr={err0}"
    );
}

#[test]
fn make_progress_orthogonal_to_format_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("blob.cfidx");
    let input = dir.path().join("in.bin");
    fs::write(&input, b"make json progress").unwrap();

    let out = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--progress",
        "--format",
        "json",
        input.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("make json");
    assert_eq!(v["ok"].as_bool(), Some(true), "{v}");
    assert!(
        !stdout.contains("progress:"),
        "progress must not land on stdout with --format json; stdout={stdout}"
    );
    assert!(
        stderr
            .lines()
            .any(|l| l.starts_with("progress: op=make done=")),
        "progress on stderr with json; stderr={stderr}"
    );
}

#[test]
fn extract_progress_emits_stderr_and_default_silent() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let src = dir.path().join("src");
    let out = dir.path().join("out");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"extract progress a").unwrap();
    fs::write(src.join("sub/b.txt"), b"extract progress b").unwrap();
    fs::write(src.join("c.txt"), b"extract progress c").unwrap();

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let with = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--progress",
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&with.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=extract done=")),
        "extract --progress stderr must contain progress: op=extract; stderr={err}"
    );
    assert!(
        err.contains("done=3/3") || err.lines().any(|l| l.contains("done=") && l.contains("/3")),
        "extract progress TOTAL should be filtered File count (3); stderr={err}"
    );

    let out2 = dir.path().join("out2");
    let without = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out2.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);
    let err0 = String::from_utf8_lossy(&without.stderr);
    assert!(
        !err0.contains("progress:"),
        "extract without --progress must not emit progress:; stderr={err0}"
    );
}

#[test]
fn extract_progress_orthogonal_to_format_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let src = dir.path().join("src");
    let out = dir.path().join("out");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"json extract a").unwrap();
    fs::write(src.join("b.txt"), b"json extract b").unwrap();

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let result = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--progress",
        "--format",
        "json",
        cfdir.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("extract json");
    assert_eq!(v["ok"].as_bool(), Some(true), "{v}");
    assert!(
        !stdout.contains("progress:"),
        "progress must not land on stdout with --format json; stdout={stdout}"
    );
    assert!(
        stderr
            .lines()
            .any(|l| l.starts_with("progress: op=extract done=")),
        "progress on stderr with json; stderr={stderr}"
    );
}

#[test]
fn extract_progress_counts_filtered_files_only() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("keep")).unwrap();
    fs::create_dir_all(src.join("skip")).unwrap();
    fs::write(src.join("keep/a.txt"), b"keep a").unwrap();
    fs::write(src.join("keep/b.txt"), b"keep b").unwrap();
    fs::write(src.join("skip/c.txt"), b"skip c").unwrap();

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let result = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        dir.path().join("out").to_str().unwrap(),
        "--progress",
        "--path",
        "keep",
        "--dry-run",
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&result.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=extract done=")),
        "dry-run --progress should still tick; stderr={err}"
    );
    assert!(
        err.contains("/2"),
        "TOTAL should be PathFilter-kept Files (2), not 3; stderr={err}"
    );
    assert!(
        !err.contains("/3"),
        "excluded file must not inflate TOTAL; stderr={err}"
    );
}

#[test]
fn extract_progress_ticks_skip_unchanged() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cfdir = dir.path().join("tree.cfdir");
    let src = dir.path().join("src");
    let out = dir.path().join("out");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"skip-unchanged a").unwrap();
    fs::write(src.join("b.txt"), b"skip-unchanged b").unwrap();

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    // First extract materializes.
    run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);
    // Second with --skip-unchanged should still tick per File judgment.
    let result = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--skip-unchanged",
        "--progress",
        cfdir.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&result.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=extract done=")),
        "skip-unchanged judgments must tick; stderr={err}"
    );
    assert!(
        err.contains("done=2/2") || err.lines().any(|l| l.contains("done=") && l.contains("/2")),
        "TOTAL=2 for two Files; stderr={err}"
    );
    assert!(
        err.contains("skipped=2"),
        "both files should skip; stderr={err}"
    );
}

// --- Phase 18 M2: --cache-stats stderr ---

#[test]
fn help_lists_cache_stats_on_cached_read_commands() {
    for cmd in ["cat", "verify", "extract", "mount", "pull", "doctor"] {
        let help = run_ok(&[cmd, "--help"]);
        let s = String::from_utf8_lossy(&help.stdout);
        assert!(
            s.contains("--cache-stats"),
            "{cmd} --help must list --cache-stats:\n{s}"
        );
        assert!(
            s.contains("--cache"),
            "{cmd} --help must list --cache:\n{s}"
        );
    }
}

#[test]
fn cache_stats_requires_cache_flag() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("out.bin");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let cases: Vec<Vec<&str>> = vec![
        vec![
            "cat",
            "--store",
            store.to_str().unwrap(),
            "--cache-stats",
            idx.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ],
        vec![
            "verify",
            "--store",
            store.to_str().unwrap(),
            "--cache-stats",
            idx.to_str().unwrap(),
        ],
    ];
    for args in cases {
        let fail = run_fail(&args);
        let err = String::from_utf8_lossy(&fail.stderr);
        assert!(
            err.contains("--cache-stats") && err.contains("--cache"),
            "args={args:?} stderr={err}"
        );
    }
}

#[test]
fn cat_cache_stats_stderr_and_second_hit_increases() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cache = dir.path().join("cache");
    let idx = dir.path().join("out.cfidx");
    let out1 = dir.path().join("out1.bin");
    let out2 = dir.path().join("out2.bin");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // First cat: miss → fill. Expect miss_fills > 0, hits typically 0.
    let first = run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "--cache-stats",
        idx.to_str().unwrap(),
        "-o",
        out1.to_str().unwrap(),
    ]);
    let err1 = String::from_utf8_lossy(&first.stderr);
    let line1 = err1
        .lines()
        .find(|l| l.starts_with("cache: hits="))
        .unwrap_or_else(|| panic!("first cat missing cache: line; stderr={err1}"));
    assert!(
        line1.contains("miss_fills=") && line1.contains("miss_refused="),
        "malformed cache line: {line1}"
    );
    let hits1 = parse_cache_hits(line1);

    // Second cat (same cache dir): should hit cache → hits > 0 (and > first).
    let second = run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "--cache-stats",
        idx.to_str().unwrap(),
        "-o",
        out2.to_str().unwrap(),
    ]);
    let err2 = String::from_utf8_lossy(&second.stderr);
    let line2 = err2
        .lines()
        .find(|l| l.starts_with("cache: hits="))
        .unwrap_or_else(|| panic!("second cat missing cache: line; stderr={err2}"));
    let hits2 = parse_cache_hits(line2);
    assert!(hits2 > 0, "second cat should report hits>0; line={line2}");
    assert!(
        hits2 > hits1,
        "second cat hits ({hits2}) should exceed first ({hits1}); line1={line1} line2={line2}"
    );
    assert_eq!(fs::read(&input).unwrap(), fs::read(&out2).unwrap());
}

#[test]
fn cat_without_cache_stats_is_quiet() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cache = dir.path().join("cache");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("out.bin");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let result = run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        idx.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&result.stderr);
    assert!(
        !err.lines().any(|l| l.starts_with("cache:")),
        "default without --cache-stats must stay quiet; stderr={err}"
    );
}

fn parse_cache_hits(line: &str) -> u64 {
    // cache: hits=H miss_fills=F miss_refused=R
    let rest = line
        .strip_prefix("cache: hits=")
        .unwrap_or_else(|| panic!("bad cache line: {line}"));
    let hits_str = rest.split_whitespace().next().unwrap_or("");
    hits_str
        .parse::<u64>()
        .unwrap_or_else(|_| panic!("bad hits in {line}"))
}

// --- Phase 18 M3: ops-json additive cache_* fields ---

fn assert_cache_ops_json_present(v: &serde_json::Value) {
    assert!(
        v.get("cache_hits").and_then(|x| x.as_u64()).is_some(),
        "missing numeric cache_hits: {v}"
    );
    assert!(
        v.get("cache_miss_fills").and_then(|x| x.as_u64()).is_some(),
        "missing numeric cache_miss_fills: {v}"
    );
    assert!(
        v.get("cache_miss_refused")
            .and_then(|x| x.as_u64())
            .is_some(),
        "missing numeric cache_miss_refused: {v}"
    );
}

fn assert_cache_ops_json_absent(v: &serde_json::Value) {
    assert!(
        v.get("cache_hits").is_none()
            && v.get("cache_miss_fills").is_none()
            && v.get("cache_miss_refused").is_none(),
        "cache_* keys must be omitted without --cache; got {v}"
    );
}

#[test]
fn cat_cache_format_json_fields_and_baseline_omit() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cache = dir.path().join("cache");
    let idx = dir.path().join("out.cfidx");
    let out_base = dir.path().join("out_base.bin");
    let out_cached = dir.path().join("out_cached.bin");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let base = run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        idx.to_str().unwrap(),
        "-o",
        out_base.to_str().unwrap(),
    ]);
    let v_base: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&base.stdout).trim()).unwrap();
    assert_eq!(v_base["ok"], true);
    assert!(v_base.get("bytes").is_some());
    assert_cache_ops_json_absent(&v_base);

    // --cache + --format json → three fields; without --cache-stats (stderr quiet).
    let cached = run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "--format",
        "json",
        idx.to_str().unwrap(),
        "-o",
        out_cached.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&cached.stderr);
    assert!(
        !err.lines().any(|l| l.starts_with("cache:")),
        "JSON cache_* must not require --cache-stats noise; stderr={err}"
    );
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&cached.stdout).trim()).unwrap();
    assert_eq!(v["ok"], true);
    assert!(v.get("bytes").is_some(), "old field bytes must remain: {v}");
    assert_cache_ops_json_present(&v);
    // First fill: expect miss_fills > 0 typically.
    assert!(
        v["cache_miss_fills"].as_u64().unwrap() > 0 || v["cache_hits"].as_u64().unwrap() > 0,
        "expected some cache activity; got {v}"
    );
}

#[test]
fn verify_cache_format_json_fields() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cache = dir.path().join("cache");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["kind"], "cfidx");
    assert!(v.get("bytes").is_some() && v.get("chunks").is_some());
    assert_cache_ops_json_present(&v);
}

#[test]
fn extract_cache_format_json_fields() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cache = dir.path().join("cache");
    let src = dir.path().join("src");
    let archive = dir.path().join("tree.cfdir");
    let out = dir.path().join("out");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"extract-cache-json\n").unwrap();

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let json_out = run_ok(&[
        "extract",
        "--store",
        store.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--format",
        "json",
        archive.to_str().unwrap(),
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&json_out.stdout).trim()).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["dry_run"], false);
    assert!(v.get("wrote").is_some() && v.get("skipped").is_some() && v.get("dirs").is_some());
    assert_cache_ops_json_present(&v);
}

#[test]
fn pull_cache_format_json_fields() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let cache = dir.path().join("cache");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let json_out = run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&json_out.stdout).trim()).unwrap();
    assert_eq!(v["ok"], true);
    assert!(v.get("fetched").is_some() && v.get("unique_chunks").is_some());
    assert_cache_ops_json_present(&v);
    assert!(
        v.get("uploaded").is_none(),
        "must not rename/break pull fields: {v}"
    );
}

#[test]
fn doctor_cache_format_json_fields() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let cache = dir.path().join("cache");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // --deep so CacheSource get-path counters move.
    let json_out = run_ok(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "--deep",
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&json_out.stdout).trim()).unwrap();
    assert_eq!(v["ok"], true);
    assert!(v.get("listings").is_some() && v.get("checked").is_some());
    assert_cache_ops_json_present(&v);
}

// --- Phase 18 M4: cat --progress ---

#[test]
fn cat_help_lists_progress() {
    let help = run_ok(&["cat", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--progress"),
        "cat --help must list --progress:\n{s}"
    );
}

#[test]
fn cat_progress_emits_stderr_and_default_silent() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("out.bin");
    let out2 = dir.path().join("out2.bin");
    // Multi-chunk blob so TOTAL > 1 is observable (avg 64KiB default still
    // yields several chunks for ~200KiB of distinct data).
    let input = dir.path().join("blob.bin");
    let mut data = Vec::with_capacity(200 * 1024);
    for i in 0..(200 * 1024) {
        data.push((i % 251) as u8);
    }
    fs::write(&input, &data).unwrap();

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let with = run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--progress",
        idx.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&with.stderr);
    assert!(
        err.lines().any(|l| l.starts_with("progress: op=cat done=")),
        "cat --progress stderr must contain progress: op=cat; stderr={err}"
    );
    assert!(
        err.contains("done=") && err.contains("/"),
        "cat progress should include done=N/TOTAL; stderr={err}"
    );
    assert_eq!(fs::read(&out).unwrap(), data);

    let without = run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
        "-o",
        out2.to_str().unwrap(),
    ]);
    let err0 = String::from_utf8_lossy(&without.stderr);
    assert!(
        !err0.contains("progress:"),
        "without --progress stderr must not contain progress:; stderr={err0}"
    );
}

#[test]
fn cat_progress_orthogonal_to_format_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("out.bin");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let result = run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        "--progress",
        "--format",
        "json",
        idx.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stderr.contains("progress: op=cat"),
        "progress on stderr with json; stderr={stderr}"
    );
    assert!(
        !stdout.contains("progress:"),
        "json stdout must not contain progress:; stdout={stdout}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("cat json invalid with --progress: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert!(v.get("bytes").is_some());
}

// --- Phase 18 M5: verify --progress ---

#[test]
fn verify_help_lists_progress() {
    let help = run_ok(&["verify", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--progress"),
        "verify --help must list --progress:\n{s}"
    );
}

#[test]
fn verify_progress_emits_stderr_and_default_silent() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    // Multi-chunk blob so TOTAL > 1 is observable.
    let input = dir.path().join("blob.bin");
    let mut data = Vec::with_capacity(200 * 1024);
    for i in 0..(200 * 1024) {
        data.push((i % 251) as u8);
    }
    fs::write(&input, &data).unwrap();

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let with = run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--progress",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&with.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=verify done=")),
        "verify --progress stderr must contain progress: op=verify; stderr={err}"
    );
    assert!(
        err.contains("done=") && err.contains("/"),
        "verify progress should include done=N/TOTAL; stderr={err}"
    );

    let without = run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let err0 = String::from_utf8_lossy(&without.stderr);
    assert!(
        !err0.contains("progress:"),
        "without --progress stderr must not contain progress:; stderr={err0}"
    );
}

#[test]
fn verify_progress_orthogonal_to_format_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let result = run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--progress",
        "--format",
        "json",
        idx.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stderr.contains("progress: op=verify"),
        "progress on stderr with json; stderr={stderr}"
    );
    assert!(
        !stdout.contains("progress:"),
        "json stdout must not contain progress:; stdout={stdout}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("verify json invalid with --progress: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["kind"], "cfidx");
}

#[test]
fn verify_cfdir_progress_chunk_granularity() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let tree = dir.path().join("tree");
    let archive = dir.path().join("tree.cfdir");
    fs::create_dir_all(&tree).unwrap();
    // Two small files → known multi-chunk TOTAL with fixed chunk size.
    fs::write(tree.join("a.bin"), vec![1u8; 8192]).unwrap();
    fs::write(tree.join("b.bin"), vec![2u8; 4096]).unwrap();

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "--chunk-size",
        "4096:4096:4096",
        "-o",
        archive.to_str().unwrap(),
        tree.to_str().unwrap(),
    ]);

    let with = run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--progress",
        archive.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&with.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=verify done=")),
        "verify --progress on cfdir must emit progress:; stderr={err}"
    );
    // Fixed 4KiB chunks: 8192+4096 → 3 chunks → last line should reach /3
    assert!(
        err.contains("/3") || err.lines().any(|l| l.contains("done=3/3")),
        "cfdir verify progress TOTAL should be chunk count; stderr={err}"
    );
}

// --- Phase 19 M1: store create ---

#[test]
fn store_create_help_lists_store_compression_format() {
    let s = run_ok(&["store", "--help"]);
    let s_out = String::from_utf8_lossy(&s.stdout);
    assert!(
        s_out.contains("create"),
        "store --help should list create:\n{s_out}"
    );

    let create = run_ok(&["store", "create", "--help"]);
    let h = String::from_utf8_lossy(&create.stdout);
    assert!(
        h.contains("--store"),
        "store create --help must list --store:\n{h}"
    );
    assert!(
        h.contains("--compression"),
        "store create --help must list --compression:\n{h}"
    );
    assert!(
        h.contains("--format"),
        "store create --help must list --format:\n{h}"
    );
}

#[test]
fn store_create_none_zstd_and_reject_duplicate() {
    let dir = tempdir().unwrap();
    let store_none = dir.path().join("s");
    let store_zstd = dir.path().join("sz");
    let store_json = dir.path().join("sj");

    let out = run_ok(&["store", "create", "--store", store_none.to_str().unwrap()]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("store create: ok") && err.contains("compression=none"),
        "text summary expected; stderr={err}"
    );
    assert!(
        out.stdout.is_empty(),
        "text mode must not write stdout; stdout={}",
        String::from_utf8_lossy(&out.stdout)
    );
    let meta = fs::read_to_string(store_none.join("meta.toml")).expect("meta.toml");
    assert!(
        meta.contains("compression = \"none\""),
        "default create must write compression=none; meta={meta}"
    );
    assert!(store_none.join("chunks").is_dir());

    let dup = run_fail(&["store", "create", "--store", store_none.to_str().unwrap()]);
    let dup_err = String::from_utf8_lossy(&dup.stderr);
    assert!(
        dup_err.contains("already exists") || dup_err.contains("store create"),
        "duplicate create must be clear non-zero; stderr={dup_err}"
    );

    run_ok(&[
        "store",
        "create",
        "--store",
        store_zstd.to_str().unwrap(),
        "--compression",
        "zstd",
    ]);
    let meta_z = fs::read_to_string(store_zstd.join("meta.toml")).expect("meta.toml zstd");
    assert!(
        meta_z.contains("compression = \"zstd\""),
        "zstd create must write compression=zstd; meta={meta_z}"
    );

    let json_out = run_ok(&[
        "store",
        "create",
        "--store",
        store_json.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let stdout = String::from_utf8_lossy(&json_out.stdout);
    let stderr = String::from_utf8_lossy(&json_out.stderr);
    assert!(
        !stderr.contains("store create: ok"),
        "json must not dual-write text summary; stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("store create json invalid: {e}; stdout={stdout}"));
    assert_eq!(v["ok"], true);
    assert_eq!(v["compression"], "none");
    assert!(
        v["store"].as_str().unwrap().contains("sj"),
        "json store path; v={v}"
    );
}

// --- Phase 19 M2: pull --compression ---

#[test]
fn pull_help_lists_compression() {
    let help = run_ok(&["pull", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--compression"),
        "pull --help must list --compression:\n{s}"
    );
    let lower = s.to_ascii_lowercase();
    assert!(
        lower.contains("none") && lower.contains("zstd"),
        "pull --help should mention none|zstd:\n{s}"
    );
    assert!(
        lower.contains("new") || lower.contains("creat"),
        "pull --help should say flag affects new stores:\n{s}"
    );
}

#[test]
fn pull_omit_compression_creates_none_store() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);

    let meta = fs::read_to_string(newstore.join("meta.toml")).expect("meta.toml");
    assert!(
        meta.contains("compression = \"none\""),
        "omit pull create must be none; meta={meta}"
    );
    let stats = run_ok(&[
        "store",
        "stats",
        "--store",
        newstore.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&stats.stdout).trim()).expect("stats json");
    assert_eq!(v["compression"].as_str(), Some("none"), "{v}");

    run_ok(&[
        "verify",
        "--store",
        newstore.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
}

#[test]
fn pull_compression_zstd_creates_zstd_store() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore-z");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--compression",
        "zstd",
        idx.to_str().unwrap(),
    ]);

    let meta = fs::read_to_string(newstore.join("meta.toml")).expect("meta.toml");
    assert!(
        meta.contains("compression = \"zstd\""),
        "pull --compression zstd must write zstd; meta={meta}"
    );
    let stats = run_ok(&[
        "store",
        "stats",
        "--store",
        newstore.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&stats.stdout).trim()).expect("stats json");
    assert_eq!(v["compression"].as_str(), Some("zstd"), "{v}");

    run_ok(&[
        "verify",
        "--store",
        newstore.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
}

#[test]
fn pull_existing_none_store_explicit_zstd_mismatch() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // First pull creates none store (omit).
    run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);

    let fail = run_fail(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--compression",
        "zstd",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.to_ascii_lowercase().contains("compression mismatch")
            || err.to_ascii_lowercase().contains("mismatch"),
        "expected mismatch error, got:\n{err}"
    );
}

#[test]
fn pull_dry_run_with_compression_does_not_create_store() {
    let dir = tempdir().unwrap();
    let local = dir.path().join("local");
    let newstore = dir.path().join("newstore-dry");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        local.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // dry-run + compression must not create meta even when store path absent.
    run_ok(&[
        "pull",
        "--store",
        newstore.to_str().unwrap(),
        "--source",
        local.to_str().unwrap(),
        "--compression",
        "zstd",
        "--dry-run",
        idx.to_str().unwrap(),
    ]);
    assert!(
        !newstore.join("meta.toml").is_file(),
        "dry-run must not create store meta.toml"
    );
    assert!(
        !newstore.exists() || !newstore.join("chunks").is_dir(),
        "dry-run must not create store layout"
    );
}

// --- Phase 19 M3: diff --progress ---

#[test]
fn diff_help_lists_progress() {
    let help = run_ok(&["diff", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(
        s.contains("--progress"),
        "diff --help must list --progress:\n{s}"
    );
}

#[test]
fn diff_progress_emits_stderr_and_default_silent() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"diff-progress-a").unwrap();
    fs::write(src.join("sub/b.txt"), b"diff-progress-b").unwrap();
    fs::write(src.join("c.txt"), b"diff-progress-c").unwrap();

    let left = dir.path().join("left.cfdir");
    let right = dir.path().join("right.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        left.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        right.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let with = run_ok(&[
        "diff",
        "--progress",
        left.to_str().unwrap(),
        right.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&with.stderr);
    let out = String::from_utf8_lossy(&with.stdout);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=diff done=")),
        "diff --progress stderr must contain progress: op=diff; stderr={err}"
    );
    // 3 File paths in union (Dir "sub" ignored) → TOTAL=3
    assert!(
        err.contains("done=3/3") || err.lines().any(|l| l.contains("done=") && l.contains("/3")),
        "diff progress TOTAL should be File path union (3); stderr={err}"
    );
    assert!(
        !out.contains("progress:"),
        "progress must not pollute stdout; stdout={out}"
    );
    assert!(
        out.contains("diff:"),
        "text summary still on stdout; stdout={out}"
    );

    let without = run_ok(&["diff", left.to_str().unwrap(), right.to_str().unwrap()]);
    let err0 = String::from_utf8_lossy(&without.stderr);
    assert!(
        !err0.contains("progress:"),
        "diff without --progress must not emit progress:; stderr={err0}"
    );
}

#[test]
fn diff_progress_orthogonal_to_format_json() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"json-diff-a").unwrap();
    fs::write(src.join("b.txt"), b"json-diff-b").unwrap();

    let left = dir.path().join("left.cfdir");
    let right = dir.path().join("right.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        left.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        right.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let out = run_ok(&[
        "diff",
        "--format",
        "json",
        "--progress",
        left.to_str().unwrap(),
        right.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("progress: op=diff"),
        "progress on stderr with json; stderr={stderr}"
    );
    assert!(
        !stdout.contains("progress:"),
        "json stdout must not contain progress:; stdout={stdout}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("diff json invalid with --progress: {e}; stdout={stdout}"));
    assert!(
        v.get("added").is_some(),
        "json must keep added; stdout={stdout}"
    );
    assert!(
        v.get("chunks_shared").is_some(),
        "json must keep chunks_shared; stdout={stdout}"
    );
}

#[test]
fn diff_tree_progress_emits_stderr() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"tree-progress-a").unwrap();
    fs::write(src.join("b.txt"), b"tree-progress-b").unwrap();

    let listing = dir.path().join("listing.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        listing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let with = run_ok(&[
        "diff",
        "--tree",
        src.to_str().unwrap(),
        "--progress",
        listing.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&with.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("progress: op=diff done=")),
        "diff --tree --progress stderr must contain progress: op=diff; stderr={err}"
    );
}

// --- Phase 19 M6: make --jobs (post-chunk put; FastCDC stays serial) ---

#[test]
fn make_help_lists_jobs_honestly() {
    let help = run_ok(&["make", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    assert!(s.contains("--jobs"), "make --help must list --jobs:\n{s}");
    let lower = s.to_lowercase();
    // Honest: post-chunk put / on-disk encoding — never claim parallel FastCDC.
    assert!(
        !lower.contains("parallel fastcdc") && !lower.contains("parallel chunking"),
        "make --help must not claim parallel FastCDC/chunking:\n{s}"
    );
    assert!(
        lower.contains("put") || lower.contains("store") || lower.contains("encoding"),
        "make --help --jobs should mention store put / encoding:\n{s}"
    );
}

#[test]
fn make_jobs_zero_rejected() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = dir.path().join("in.bin");
    fs::write(&input, b"make-jobs-zero").unwrap();
    let out = run_fail(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--jobs",
        "0",
        input.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("jobs") && (err.contains(">= 1") || err.contains("0")),
        "stderr={err}"
    );
}

#[test]
fn make_jobs_one_matches_default_and_jobs_four_cfidx() {
    let dir = tempdir().unwrap();
    // Multi-chunk patterned blob so put parallelism is exercised.
    let mut data = Vec::with_capacity(48 * 1024);
    for i in 0..(48 * 1024) {
        data.push(((i * 17 + 3) % 251) as u8);
    }
    let input = dir.path().join("multi.bin");
    fs::write(&input, &data).unwrap();
    let chunk_size = "2048:4096:8192";

    let store_def = dir.path().join("store_def");
    let store1 = dir.path().join("store1");
    let store4 = dir.path().join("store4");
    let out_def = dir.path().join("def.cfidx");
    let out1 = dir.path().join("j1.cfidx");
    let out4 = dir.path().join("j4.cfidx");

    let def = run_ok(&[
        "make",
        "--store",
        store_def.to_str().unwrap(),
        "-o",
        out_def.to_str().unwrap(),
        "--chunk-size",
        chunk_size,
        input.to_str().unwrap(),
    ]);
    let j1 = run_ok(&[
        "make",
        "--store",
        store1.to_str().unwrap(),
        "-o",
        out1.to_str().unwrap(),
        "--chunk-size",
        chunk_size,
        "--jobs",
        "1",
        input.to_str().unwrap(),
    ]);
    let j4 = run_ok(&[
        "make",
        "--store",
        store4.to_str().unwrap(),
        "-o",
        out4.to_str().unwrap(),
        "--chunk-size",
        chunk_size,
        "--jobs",
        "4",
        "--format",
        "json",
        input.to_str().unwrap(),
    ]);

    let b_def = fs::read(&out_def).unwrap();
    let b1 = fs::read(&out1).unwrap();
    let b4 = fs::read(&out4).unwrap();
    assert_eq!(b_def, b1, "default (no --jobs) ≡ --jobs 1 .cfidx bytes");
    assert_eq!(
        b1, b4,
        "make --jobs 1 and --jobs 4 must produce identical .cfidx bytes"
    );

    let err_def = String::from_utf8_lossy(&def.stderr);
    let err1 = String::from_utf8_lossy(&j1.stderr);
    let (new_def, reused_def) = parse_make_stats(&err_def);
    let (new1, reused1) = parse_make_stats(&err1);
    assert_eq!(
        (new_def, reused_def),
        (new1, reused1),
        "default ≡ jobs=1 new/reused; def={err_def} j1={err1}"
    );
    assert_eq!(reused_def, 0, "fresh store reused=0; stderr={err_def}");
    assert!(new_def >= 4, "expect multi-chunk; stderr={err_def}");

    let stdout4 = String::from_utf8_lossy(&j4.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout4.trim())
        .unwrap_or_else(|e| panic!("make --jobs 4 json invalid: {e}; stdout={stdout4}"));
    let chunks = v["chunks"].as_u64().expect("chunks") as usize;
    let new4 = v["new"].as_u64().expect("new") as usize;
    let reused4 = v["reused"].as_u64().expect("reused") as usize;
    assert_eq!(reused4, 0, "fresh store jobs=4 reused=0; json={stdout4}");
    assert_eq!(
        new1 + reused1,
        new4 + reused4,
        "jobs=1 and jobs=4 new+reused must match; j1={err1} j4={stdout4}"
    );
    assert_eq!(
        new4 + reused4,
        chunks,
        "new+reused must equal chunks; json={stdout4}"
    );

    run_ok(&[
        "verify",
        "--store",
        store4.to_str().unwrap(),
        out4.to_str().unwrap(),
    ]);
}

// --- Phase 20 M2: --path-from on archive/extract/push/pull/diff ---

#[test]
fn path_from_help_on_archive_extract_push_pull_diff() {
    for cmd in ["archive", "extract", "push", "pull", "diff"] {
        let help = run_ok(&[cmd, "--help"]);
        let s = String::from_utf8_lossy(&help.stdout);
        assert!(
            s.contains("--path-from"),
            "{cmd} --help should list --path-from:\n{s}"
        );
    }
}

#[test]
fn archive_path_from_include_matches_cli_path() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("a")).unwrap();
    fs::create_dir_all(src.join("b")).unwrap();
    fs::write(src.join("a").join("f.txt"), b"hello-a\n").unwrap();
    fs::write(src.join("b").join("g.txt"), b"hello-b\n").unwrap();

    let store = dir.path().join("store");
    let via_path = dir.path().join("via_path.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        via_path.to_str().unwrap(),
        "--path",
        "a",
        src.to_str().unwrap(),
    ]);
    let bytes = fs::read(&via_path).unwrap();
    let arch_path = chunkforge_index::DirArchive::decode(&bytes).unwrap();
    let paths_cli: Vec<_> = arch_path.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths_cli,
        vec!["a/f.txt"],
        "cli --path; paths={paths_cli:?}"
    );

    let include = dir.path().join("include.txt");
    fs::write(&include, "# only a\n\n  a  \n").unwrap();
    let via_from = dir.path().join("via_from.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        via_from.to_str().unwrap(),
        "--path-from",
        include.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let bytes = fs::read(&via_from).unwrap();
    let arch_from = chunkforge_index::DirArchive::decode(&bytes).unwrap();
    let paths_from: Vec<_> = arch_from.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths_from, paths_cli,
        "path-from must match handwritten --path; from={paths_from:?} cli={paths_cli:?}"
    );

    // Merge: CLI --path OR path-from
    let include_b = dir.path().join("include_b.txt");
    fs::write(&include_b, "b\n").unwrap();
    let merged = dir.path().join("merged.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        merged.to_str().unwrap(),
        "--path",
        "a",
        "--path-from",
        include_b.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let bytes = fs::read(&merged).unwrap();
    let arch = chunkforge_index::DirArchive::decode(&bytes).unwrap();
    let mut paths: Vec<_> = arch.entries.iter().map(|e| e.path.clone()).collect();
    paths.sort();
    assert_eq!(
        paths,
        vec!["a/f.txt".to_string(), "b/g.txt".to_string()],
        "OR merge paths={paths:?}"
    );
}

#[test]
fn archive_path_from_missing_file_nonzero() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"x\n").unwrap();
    let store = dir.path().join("store");
    let out = dir.path().join("app.cfdir");
    let missing = dir.path().join("no-such-paths.txt");

    let fail = run_fail(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--path-from",
        missing.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("path file") || err.contains("cannot read") || err.contains("PathFile"),
        "missing path-from file should be a clear non-zero error; stderr={err}"
    );
    assert!(!out.exists(), "must not write .cfdir on path-from error");
}

#[test]
fn extract_push_pull_diff_path_from_missing_file_nonzero() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("missing-paths.txt");
    let listing = dir.path().join("nope.cfdir");
    let store = dir.path().join("store");

    for args in [
        vec![
            "extract".to_string(),
            "--store".into(),
            store.display().to_string(),
            "-o".into(),
            dir.path().join("out").display().to_string(),
            "--path-from".into(),
            missing.display().to_string(),
            listing.display().to_string(),
        ],
        vec![
            "pull".to_string(),
            "--store".into(),
            store.display().to_string(),
            "--source".into(),
            store.display().to_string(),
            "--path-from".into(),
            missing.display().to_string(),
            listing.display().to_string(),
        ],
        vec![
            "push".to_string(),
            "--store".into(),
            store.display().to_string(),
            "--dest".into(),
            "http://127.0.0.1:9".into(),
            "--path-from".into(),
            missing.display().to_string(),
            listing.display().to_string(),
        ],
        vec![
            "diff".to_string(),
            "--path-from".into(),
            missing.display().to_string(),
            listing.display().to_string(),
            listing.display().to_string(),
        ],
    ] {
        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let fail = run_fail(&args_ref);
        let err = String::from_utf8_lossy(&fail.stderr);
        assert!(
            err.contains("path file") || err.contains("cannot read") || err.contains("PathFile"),
            "cmd {:?} missing path-from should fail clearly; stderr={err}",
            args[0]
        );
    }
}

#[test]
fn push_cfidx_with_path_from_errors_clearly() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let include = dir.path().join("inc.txt");
    fs::write(&include, "a\n").unwrap();
    let fail = run_fail(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        "http://127.0.0.1:9",
        "--path-from",
        include.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("--path") || err.contains("looks like a `.cfidx`") || err.contains("cfidx"),
        "stderr should explain cfidx+path-from is invalid; stderr={err}"
    );
}

// --- Phase 20 M3: doctor/verify path scope ---

#[test]
fn doctor_verify_help_lists_path_four_flags() {
    for cmd in ["doctor", "verify"] {
        let help = run_ok(&[cmd, "--help"]);
        let s = String::from_utf8_lossy(&help.stdout);
        for flag in ["--path", "--path-from", "--exclude", "--exclude-from"] {
            assert!(s.contains(flag), "{cmd} --help should list {flag}:\n{s}");
        }
    }
}

#[test]
fn gc_help_has_no_path_flags() {
    let help = run_ok(&["gc", "--help"]);
    let s = String::from_utf8_lossy(&help.stdout);
    // Avoid matching "--path" inside longer words; clap prints "  --path ".
    assert!(
        !s.contains("--path ")
            && !s.contains("--path\n")
            && !s.contains("--path-from")
            && !s.contains("--exclude")
            && !s.contains("--exclude-from"),
        "gc --help must NOT list path/exclude flags:\n{s}"
    );
}

#[test]
fn doctor_cfidx_plus_path_nonzero() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let fail = run_fail(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        "--path",
        "a",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("--path") || err.contains("looks like a `.cfidx`") || err.contains("cfidx"),
        "doctor .cfidx+--path must be clear non-zero; stderr={err}"
    );
}

#[test]
fn verify_cfidx_plus_path_nonzero() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("hello.cfidx");
    let input = fixtures_dir().join("hello.txt");
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    let fail = run_fail(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--path",
        "a",
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&fail.stderr);
    assert!(
        err.contains("--path") || err.contains("looks like a `.cfidx`") || err.contains("cfidx"),
        "verify .cfidx+--path must be clear non-zero; stderr={err}"
    );
}

#[test]
fn doctor_cfdir_path_subset_shrinks_checked() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("keep")).unwrap();
    fs::create_dir_all(src.join("skip")).unwrap();
    fs::write(src.join("keep").join("a.txt"), b"keep-doctor-aaa\n").unwrap();
    fs::write(src.join("skip").join("b.txt"), b"skip-doctor-bbb\n").unwrap();
    let cfdir = dir.path().join("tree.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let full = run_ok(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        cfdir.to_str().unwrap(),
    ]);
    let full_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&full.stdout).trim()).unwrap();
    let full_checked = full_v["checked"].as_u64().unwrap();
    assert!(
        full_checked >= 2,
        "full doctor should check both files' chunks; {full_v}"
    );

    let subset = run_ok(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        "--path",
        "keep",
        cfdir.to_str().unwrap(),
    ]);
    let sub_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&subset.stdout).trim()).unwrap();
    assert_eq!(sub_v["ok"], true);
    let sub_checked = sub_v["checked"].as_u64().unwrap();
    assert!(
        sub_checked < full_checked && sub_checked >= 1,
        "subset checked must shrink: full={full_checked} sub={sub_checked}; {sub_v}"
    );
}

#[test]
fn verify_cfdir_path_subset_shrinks_files() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("keep")).unwrap();
    fs::create_dir_all(src.join("skip")).unwrap();
    fs::write(src.join("keep").join("a.txt"), b"keep-verify-aaa\n").unwrap();
    fs::write(src.join("skip").join("b.txt"), b"skip-verify-bbb\n").unwrap();
    let cfdir = dir.path().join("tree.cfdir");

    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let full = run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        cfdir.to_str().unwrap(),
    ]);
    let full_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&full.stdout).trim()).unwrap();
    assert_eq!(full_v["ok"], true);
    assert_eq!(full_v["kind"], "cfdir");
    let full_files = full_v["files"].as_u64().unwrap();
    let full_chunks = full_v["chunks"].as_u64().unwrap();
    assert_eq!(full_files, 2, "full verify files; {full_v}");

    let subset = run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        "--path",
        "keep",
        cfdir.to_str().unwrap(),
    ]);
    let sub_v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&subset.stdout).trim()).unwrap();
    assert_eq!(sub_v["ok"], true);
    assert_eq!(sub_v["kind"], "cfdir");
    let sub_files = sub_v["files"].as_u64().unwrap();
    let sub_chunks = sub_v["chunks"].as_u64().unwrap();
    assert_eq!(sub_files, 1, "subset verify files; {sub_v}");
    assert!(
        sub_chunks < full_chunks && sub_chunks >= 1,
        "subset chunks must shrink: full={full_chunks} sub={sub_chunks}; {sub_v}"
    );
}

#[test]
fn doctor_verify_path_from_and_exclude_from_smoke() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("a")).unwrap();
    fs::create_dir_all(src.join("b")).unwrap();
    fs::write(src.join("a").join("f.txt"), b"path-from-a\n").unwrap();
    fs::write(src.join("b").join("g.txt"), b"path-from-b\n").unwrap();
    let cfdir = dir.path().join("tree.cfdir");
    run_ok(&[
        "archive",
        "--store",
        store.to_str().unwrap(),
        "-o",
        cfdir.to_str().unwrap(),
        src.to_str().unwrap(),
    ]);

    let include = dir.path().join("inc.txt");
    fs::write(&include, "# only a\na\n").unwrap();
    let exclude = dir.path().join("exc.txt");
    fs::write(&exclude, "b/\n").unwrap();

    let out = run_ok(&[
        "doctor",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        "--path-from",
        include.to_str().unwrap(),
        "--exclude-from",
        exclude.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(v["ok"], true);
    assert!(v["checked"].as_u64().unwrap() >= 1, "{v}");

    let out = run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        "--format",
        "json",
        "--path-from",
        include.to_str().unwrap(),
        cfdir.to_str().unwrap(),
    ]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["files"], 1, "{v}");
}
