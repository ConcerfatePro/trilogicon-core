//! Shared helpers for subprocess integration tests (`CARGO_BIN_EXE_node`).
#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use node::genesis::{Genesis, GenesisAllocation};

pub fn node_exe() -> PathBuf {
    option_env!("CARGO_BIN_EXE_node")
        .expect("CARGO_BIN_EXE_node must be set (run via cargo test)")
        .into()
}

pub fn temp_workdir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!(
        "trilogicon_e2e_{}_{}_{}",
        label,
        std::process::id(),
        nanos
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

pub fn parse_init_address(stdout: &str) -> String {
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("Address: ") {
            return rest.trim().to_string();
        }
    }
    panic!("no 'Address:' line in init stdout:\n{stdout}");
}

/// Spawn a thread that reads all stdout lines; return the `network: listening on …` address before deadline.
pub fn wait_listen_addr(out: impl std::io::Read + Send + 'static, deadline: Duration) -> String {
    let (found_tx, found_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else {
                break;
            };
            if let Some(rest) = line.strip_prefix("network: listening on ") {
                let addr = rest.trim().to_string();
                let _ = found_tx.send(addr);
            }
        }
    });
    let start = Instant::now();
    loop {
        if start.elapsed() > deadline {
            panic!("timeout waiting for listener address");
        }
        match found_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(addr) => return addr,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("stdout reader ended before listener line");
            }
        }
    }
}

pub struct TwoNodeGenesis {
    pub bin: PathBuf,
    pub root: PathBuf,
    pub dir_a: PathBuf,
    pub dir_b: PathBuf,
    pub merged: Genesis,
    pub addr_a: String,
    pub addr_b: String,
}

/// `init` both dirs, merge allocations into `genesis.toml` on each side (10M TRIL each).
pub fn setup_two_node_genesis(label: &str) -> TwoNodeGenesis {
    let bin = node_exe();
    let root = temp_workdir(label);
    let dir_a = root.join("a");
    let dir_b = root.join("b");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_b).unwrap();

    let out_a = Command::new(&bin)
        .args([
            "init",
            "--data-dir",
            dir_a.to_str().unwrap(),
            "--genesis-balance",
            "10000000",
        ])
        .output()
        .expect("spawn init a");
    assert!(
        out_a.status.success(),
        "init a: {}",
        String::from_utf8_lossy(&out_a.stderr)
    );
    let addr_a = parse_init_address(&String::from_utf8_lossy(&out_a.stdout));

    let out_b = Command::new(&bin)
        .args(["init", "--data-dir", dir_b.to_str().unwrap()])
        .output()
        .expect("spawn init b");
    assert!(
        out_b.status.success(),
        "init b: {}",
        String::from_utf8_lossy(&out_b.stderr)
    );
    let addr_b = parse_init_address(&String::from_utf8_lossy(&out_b.stdout));

    let merged = Genesis {
        allocations: vec![
            GenesisAllocation {
                address: addr_a.clone(),
                balance: 10_000_000,
            },
            GenesisAllocation {
                address: addr_b.clone(),
                balance: 10_000_000,
            },
        ],
    };
    merged
        .write_to_path(&dir_a.join("genesis.toml"))
        .expect("write genesis a");
    merged
        .write_to_path(&dir_b.join("genesis.toml"))
        .expect("write genesis b");

    TwoNodeGenesis {
        bin,
        root,
        dir_a,
        dir_b,
        merged,
        addr_a,
        addr_b,
    }
}

/// Three data dirs + merged genesis (10M each). **A** is funded at init; **B** and **C** are plain `init`.
pub struct ThreeNodeGenesis {
    pub bin: PathBuf,
    pub root: PathBuf,
    pub dir_a: PathBuf,
    pub dir_b: PathBuf,
    pub dir_c: PathBuf,
    pub merged: Genesis,
    pub addr_a: String,
    pub addr_b: String,
    pub addr_c: String,
}

/// Same merge pattern as two-node; third party gets its own allocation in `genesis.toml`.
pub fn setup_three_node_genesis(label: &str) -> ThreeNodeGenesis {
    let bin = node_exe();
    let root = temp_workdir(label);
    let dir_a = root.join("a");
    let dir_b = root.join("b");
    let dir_c = root.join("c");
    for d in [&dir_a, &dir_b, &dir_c] {
        std::fs::create_dir_all(d).unwrap();
    }

    let out_a = Command::new(&bin)
        .args([
            "init",
            "--data-dir",
            dir_a.to_str().unwrap(),
            "--genesis-balance",
            "10000000",
        ])
        .output()
        .expect("spawn init a");
    assert!(
        out_a.status.success(),
        "init a: {}",
        String::from_utf8_lossy(&out_a.stderr)
    );
    let addr_a = parse_init_address(&String::from_utf8_lossy(&out_a.stdout));

    let out_b = Command::new(&bin)
        .args(["init", "--data-dir", dir_b.to_str().unwrap()])
        .output()
        .expect("spawn init b");
    assert!(
        out_b.status.success(),
        "init b: {}",
        String::from_utf8_lossy(&out_b.stderr)
    );
    let addr_b = parse_init_address(&String::from_utf8_lossy(&out_b.stdout));

    let out_c = Command::new(&bin)
        .args(["init", "--data-dir", dir_c.to_str().unwrap()])
        .output()
        .expect("spawn init c");
    assert!(
        out_c.status.success(),
        "init c: {}",
        String::from_utf8_lossy(&out_c.stderr)
    );
    let addr_c = parse_init_address(&String::from_utf8_lossy(&out_c.stdout));

    let merged = Genesis {
        allocations: vec![
            GenesisAllocation {
                address: addr_a.clone(),
                balance: 10_000_000,
            },
            GenesisAllocation {
                address: addr_b.clone(),
                balance: 10_000_000,
            },
            GenesisAllocation {
                address: addr_c.clone(),
                balance: 10_000_000,
            },
        ],
    };
    for d in [&dir_a, &dir_b, &dir_c] {
        merged
            .write_to_path(&d.join("genesis.toml"))
            .expect("write genesis");
    }

    ThreeNodeGenesis {
        bin,
        root,
        dir_a,
        dir_b,
        dir_c,
        merged,
        addr_a,
        addr_b,
        addr_c,
    }
}
