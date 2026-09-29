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
}

#[test]
fn push_rejects_non_http_dest_and_template_flags() {
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
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("http") && (err.contains("dest") || err.contains("--dest")),
        "stderr={err}"
    );

    let out = run_fail(&[
        "push",
        "--store",
        store.to_str().unwrap(),
        "--dest",
        store.to_str().unwrap(),
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

    // Ensure credentials are absent for this process invocation.
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
