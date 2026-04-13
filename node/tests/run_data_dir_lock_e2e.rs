//! Subprocess E2E: a second `node run` on the same `--data-dir` must fail-closed on `.node.run.lock`.
//!
//! Requires `CARGO_BIN_EXE_node` (set automatically by `cargo test`).

mod e2e_common;

use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use e2e_common::setup_two_node_genesis;

#[test]
fn second_run_exits_fail_closed_when_run_lock_held() {
    let g = setup_two_node_genesis("run_dir_lock");
    let bin = &g.bin;
    let dir = &g.dir_a;

    let mut holder = Command::new(bin)
        .args([
            "run",
            "--data-dir",
            dir.to_str().unwrap(),
            "--interval-secs",
            "30",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn first run");

    thread::sleep(Duration::from_millis(800));

    let dup = Command::new(bin)
        .args([
            "run",
            "--data-dir",
            dir.to_str().unwrap(),
            "--interval-secs",
            "2",
        ])
        .output()
        .expect("spawn second run");

    assert!(
        !dup.status.success(),
        "second run should exit non-zero:\nstderr={}",
        String::from_utf8_lossy(&dup.stderr)
    );
    let err = String::from_utf8_lossy(&dup.stderr);
    assert!(
        err.contains("fail-closed") && err.contains("another `node run`"),
        "expected run lock fail-closed message, got:\n{err}"
    );

    let _ = holder.kill();
    let _ = holder.wait();
}
