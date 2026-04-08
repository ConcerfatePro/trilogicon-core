//! Subprocess E2E: `--handshake`, `--exchange-peers`, `--announce-blocks`, and strict inbound HELLO.
//!
//! - B seeds an unreachable peer so `REQUEST_PEERS` returns something A can merge.
//! - After inbound HELLO, B should record A's socket in `peer_book.toml` (>= 2 `addr` lines).

mod e2e_common;

use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use e2e_common::{setup_two_node_genesis, wait_listen_addr};
use node::storage::load_blockchain_from_disk;

fn count_addr_entries(peer_book_toml: &str) -> usize {
    peer_book_toml.matches("addr =").count()
}

#[test]
fn e2e_gossip_extensions_handshake_chains_and_peer_exchange() {
    let g = setup_two_node_genesis("gossip_ext");
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
            "--peers",
            "127.0.0.1:9",
            "--interval-secs",
            "1",
            "--require-handshake-inbound",
            "--no-legacy-inbound",
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
            "--handshake",
            "--exchange-peers",
            "--announce-blocks",
            "--require-handshake-inbound",
            "--no-legacy-inbound",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
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

    thread::sleep(Duration::from_secs(10));

    let _ = child_a.kill();
    let _ = child_b.kill();
    let _ = child_a.wait();
    let _ = child_b.wait();

    let (chain_a, _) =
        load_blockchain_from_disk(dir_a.join("chain.blocks"), &g.merged).expect("load a");
    let (chain_b, _) =
        load_blockchain_from_disk(dir_b.join("chain.blocks"), &g.merged).expect("load b");

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

    let book_a = std::fs::read_to_string(dir_a.join("peer_book.toml")).expect("read peer_book a");
    assert!(
        book_a.contains("127.0.0.1:9"),
        "A should merge peer list from B (seed 127.0.0.1:9); got:\n{book_a}"
    );

    let book_b = std::fs::read_to_string(dir_b.join("peer_book.toml")).expect("read peer_book b");
    assert!(
        count_addr_entries(&book_b) >= 2,
        "B should record inbound HELLO peer plus seed; expected >= 2 addr lines, got:\n{book_b}"
    );

    let _ = std::fs::remove_dir_all(&g.root);
}
