//! Subprocess E2E: three nodes, **fan-out** gossip (producer A pushes sealed blocks to B and C).
//!
//! Current `run` only gossips to `--peers` on the **sealer**; it does not re-forward blocks received
//! from another peer. So this test uses one producer with `--peers B,C` and two listeners.
//!
//! Requires `CARGO_BIN_EXE_node` (set automatically by `cargo test`).

mod e2e_common;

use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use e2e_common::{setup_three_node_genesis, wait_listen_addr};
use node::storage::load_blockchain_from_disk;

#[test]
fn e2e_three_nodes_fan_out_send_match_chains() {
    let g = setup_three_node_genesis("fanout");
    let bin = &g.bin;

    let mut child_b = Command::new(bin)
        .args([
            "run",
            "--data-dir",
            g.dir_b.to_str().unwrap(),
            "--listen",
            "127.0.0.1:0",
            "--interval-secs",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn run b");

    let peer_b = wait_listen_addr(
        child_b.stdout.take().expect("b stdout"),
        Duration::from_secs(15),
    );

    let mut child_c = Command::new(bin)
        .args([
            "run",
            "--data-dir",
            g.dir_c.to_str().unwrap(),
            "--listen",
            "127.0.0.1:0",
            "--interval-secs",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn run c");

    let peer_c = wait_listen_addr(
        child_c.stdout.take().expect("c stdout"),
        Duration::from_secs(15),
    );

    let peers = format!("{peer_b},{peer_c}");

    let mut child_a = Command::new(bin)
        .args([
            "run",
            "--data-dir",
            g.dir_a.to_str().unwrap(),
            "--peers",
            &peers,
            "--interval-secs",
            "1",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn run a");

    thread::sleep(Duration::from_millis(600));

    assert!(
        Command::new(bin)
            .args([
                "send",
                "--data-dir",
                g.dir_a.to_str().unwrap(),
                &g.addr_c,
                "100",
                "1",
            ])
            .status()
            .expect("send")
            .success(),
        "send failed"
    );

    thread::sleep(Duration::from_secs(10));

    let _ = child_a.kill();
    let _ = child_b.kill();
    let _ = child_c.kill();
    let _ = child_a.wait();
    let _ = child_b.wait();
    let _ = child_c.wait();

    let (chain_a, _) =
        load_blockchain_from_disk(g.dir_a.join("chain.blocks"), &g.merged).expect("load a");
    let (chain_b, _) =
        load_blockchain_from_disk(g.dir_b.join("chain.blocks"), &g.merged).expect("load b");
    let (chain_c, _) =
        load_blockchain_from_disk(g.dir_c.join("chain.blocks"), &g.merged).expect("load c");

    assert!(
        chain_a.height() >= 1,
        "expected height >= 1 on A, got {}",
        chain_a.height()
    );
    assert_eq!(chain_a.height(), chain_b.height(), "A vs B height");
    assert_eq!(chain_a.height(), chain_c.height(), "A vs C height");
    let tip = chain_a.blocks().last().unwrap().block_hash.clone();
    assert_eq!(chain_b.blocks().last().unwrap().block_hash, tip);
    assert_eq!(chain_c.blocks().last().unwrap().block_hash, tip);
    assert_eq!(
        chain_a.state().accounts_sorted(),
        chain_b.state().accounts_sorted()
    );
    assert_eq!(
        chain_a.state().accounts_sorted(),
        chain_c.state().accounts_sorted()
    );

    let _ = std::fs::remove_dir_all(&g.root);
}
