//! Subprocess E2E: two nodes, merged genesis, **restart** same data dirs after kill, then second `send`.
//!
//! Requires `CARGO_BIN_EXE_node` (set automatically by `cargo test`).

mod e2e_common;

use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use e2e_common::{setup_two_node_genesis, wait_listen_addr};
use node::storage::load_blockchain_from_disk;

fn spawn_run_pair(
    bin: &std::path::Path,
    dir_a: &std::path::Path,
    dir_b: &std::path::Path,
) -> (std::process::Child, std::process::Child, String) {
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

    let child_a = Command::new(bin)
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

    (child_a, child_b, peer_b)
}

#[test]
fn e2e_two_nodes_restart_and_continue_matching_chains() {
    let g = setup_two_node_genesis("restart");
    let bin = &g.bin;
    let dir_a = &g.dir_a;
    let dir_b = &g.dir_b;

    let (mut child_a, mut child_b, _peer) = spawn_run_pair(bin, dir_a, dir_b);
    thread::sleep(Duration::from_millis(500));

    assert!(
        Command::new(bin)
            .args([
                "send",
                "--data-dir",
                dir_a.to_str().unwrap(),
                &g.addr_b,
                "100",
                "1",
            ])
            .status()
            .expect("send 1")
            .success(),
        "first send failed"
    );

    thread::sleep(Duration::from_secs(8));

    let _ = child_a.kill();
    let _ = child_b.kill();
    let _ = child_a.wait();
    let _ = child_b.wait();

    let (chain_a_after_1, _) =
        load_blockchain_from_disk(dir_a.join("chain.blocks"), &g.merged).expect("load a pass1");
    let (chain_b_after_1, _) =
        load_blockchain_from_disk(dir_b.join("chain.blocks"), &g.merged).expect("load b pass1");

    assert!(
        chain_a_after_1.height() >= 1,
        "expected height >= 1 after first run, got {}",
        chain_a_after_1.height()
    );
    assert_eq!(chain_a_after_1.height(), chain_b_after_1.height());
    assert_eq!(
        chain_a_after_1.blocks().last().unwrap().block_hash,
        chain_b_after_1.blocks().last().unwrap().block_hash
    );

    let height_after_first = chain_a_after_1.height();

    // Second run: reload chain + wallet + genesis from disk; exercise persistence path.
    let (mut child_a2, mut child_b2, _) = spawn_run_pair(bin, dir_a, dir_b);
    thread::sleep(Duration::from_millis(500));

    assert!(
        Command::new(bin)
            .args([
                "send",
                "--data-dir",
                dir_a.to_str().unwrap(),
                &g.addr_b,
                "50",
                "1",
            ])
            .status()
            .expect("send 2")
            .success(),
        "second send failed"
    );

    thread::sleep(Duration::from_secs(10));

    let _ = child_a2.kill();
    let _ = child_b2.kill();
    let _ = child_a2.wait();
    let _ = child_b2.wait();

    let (chain_a_final, _) =
        load_blockchain_from_disk(dir_a.join("chain.blocks"), &g.merged).expect("load a final");
    let (chain_b_final, _) =
        load_blockchain_from_disk(dir_b.join("chain.blocks"), &g.merged).expect("load b final");

    assert!(
        chain_a_final.height() >= height_after_first,
        "expected chain to grow or stay same after restart+send, was {} now {}",
        height_after_first,
        chain_a_final.height()
    );
    assert_eq!(chain_a_final.height(), chain_b_final.height());
    assert_eq!(
        chain_a_final.blocks().last().unwrap().block_hash,
        chain_b_final.blocks().last().unwrap().block_hash
    );
    assert_eq!(
        chain_a_final.state().accounts_sorted(),
        chain_b_final.state().accounts_sorted(),
        "state mismatch after restart"
    );

    let _ = std::fs::remove_dir_all(&g.root);
}
