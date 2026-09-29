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
