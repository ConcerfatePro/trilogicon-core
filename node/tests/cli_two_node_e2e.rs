//! Subprocess E2E: real `node` binary, two data dirs, merged genesis, listener + producer + send.
//!
//! Requires `CARGO_BIN_EXE_node` (set automatically by `cargo test`).

mod e2e_common;

use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use e2e_common::{setup_two_node_genesis, wait_listen_addr};
use node::storage::load_blockchain_from_disk;

#[test]
fn e2e_two_nodes_merge_genesis_send_and_match_chains() {
    let g = setup_two_node_genesis("basic");
    let bin = &g.bin;
    let dir_a = &g.dir_a;
    let dir_b = &g.dir_b;

    let mut child_b = Command::new(bin)
        .args([
            "run",
            "--data-dir",
            dir_b.to_str().unwrap(),
            "--listen",
            "127.0.0.1:0",
            "--interval-secs",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn run b");

    let b_stdout = child_b.stdout.take().expect("b stdout");
    let peer_b = wait_listen_addr(b_stdout, Duration::from_secs(15));

    let mut child_a = Command::new(bin)
        .args([
            "run",
            "--data-dir",
            dir_a.to_str().unwrap(),
            "--listen",
            "127.0.0.1:0",
            "--peers",
            &peer_b,
            "--interval-secs",
            "1",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn run a");

    thread::sleep(Duration::from_millis(500));

    let send_status = Command::new(bin)
        .args([
            "send",
            "--data-dir",
            dir_a.to_str().unwrap(),
            &g.addr_b,
            "100",
            "1",
        ])
        .status()
        .expect("spawn send");
    assert!(send_status.success(), "send failed");

    thread::sleep(Duration::from_secs(8));

    let _ = child_a.kill();
    let _ = child_b.kill();
    let _ = child_a.wait();
    let _ = child_b.wait();

    let chain_a = load_blockchain_from_disk(dir_a.join("chain.blocks"), &g.merged).expect("load a");
    let chain_b = load_blockchain_from_disk(dir_b.join("chain.blocks"), &g.merged).expect("load b");

    assert!(
        chain_a.height() >= 1,
        "expected at least one block on A, height={}",
        chain_a.height()
    );
    assert_eq!(chain_a.height(), chain_b.height(), "height mismatch");
    assert_eq!(
        chain_a.blocks().last().unwrap().block_hash,
        chain_b.blocks().last().unwrap().block_hash,
        "tip hash mismatch"
    );
    assert_eq!(
        chain_a.state().accounts_sorted(),
        chain_b.state().accounts_sorted(),
        "state mismatch"
    );

    let _ = std::fs::remove_dir_all(&g.root);
}
