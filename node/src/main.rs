//! Dev CLI: create a wallet, queue signed transfers, run a minimal block producer.
//!
//! ```text
//! cargo run -p node -- init [--data-dir DIR]
//! cargo run -p node -- run [--data-dir DIR] [--interval-secs SECS]
//!   [--listen HOST:PORT] [--peers A,B] [--max-future-drift-secs N]
//! cargo run -p node -- send [--data-dir DIR] RECEIVER AMOUNT [FEE]
//! ```
//!
//! `send` writes to `pending_tx.tril`; `run` ingests that file into the mempool and seals blocks.
//! Chain frames go to `chain.blocks`. `wallet.seed` holds 32 raw bytes (back it up).

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use node::blockchain::Blockchain;
use node::encoding::{decode_transaction, encode_transaction};
use node::mempool::Mempool;
use node::network::NodeInner;
use node::storage::{load_blockchain_from_disk_with, BlockStore};
use node::transaction::Transaction;
use node::types::Address;
use node::wallet::Wallet;

const WALLET_FILE: &str = "wallet.seed";
const CHAIN_FILE: &str = "chain.blocks";
const PENDING_TX_FILE: &str = "pending_tx.tril";
/// Idempotent dev funding so a fresh wallet can pay fees without a genesis-state file yet.
const DEV_BOOTSTRAP_BALANCE: u64 = 1_000_000;
const DEFAULT_INTERVAL_SECS: u64 = 2;
const MAX_PENDING_TX_FRAME: u32 = 4 * 1024 * 1024;

fn usage(bin: &str) {
    eprintln!(
        "Usage:
  {bin} init [--data-dir DIR]
  {bin} run [--data-dir DIR] [--interval-secs SECS]
          [--listen HOST:PORT] [--peers HOST:PORT,...] [--max-future-drift-secs N]
  {bin} send [--data-dir DIR] RECEIVER AMOUNT [FEE]

Files under DIR (default \".\"): {WALLET_FILE}, {CHAIN_FILE}, {PENDING_TX_FILE}"
    );
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn dev_bootstrap(wallet: &Wallet, chain: &mut Blockchain) {
    chain
        .state_mut()
        .create_account(wallet.address(), DEV_BOOTSTRAP_BALANCE);
}

fn load_wallet(data_dir: &Path) -> std::io::Result<Wallet> {
    let path = data_dir.join(WALLET_FILE);
    let raw = fs::read(&path).map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            std::io::Error::new(
                ErrorKind::NotFound,
                format!(
                    "{}: no wallet here — run `init --data-dir {}` first (same folder as send/run)",
                    path.display(),
                    data_dir.display()
                ),
            )
        } else {
            e
        }
    })?;
    if raw.len() != 32 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("expected 32-byte {}, got {} bytes", path.display(), raw.len()),
        ));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&raw);
    Ok(Wallet::from_seed(&seed))
}

fn append_pending_tx(path: &Path, tx: &Transaction) -> std::io::Result<()> {
    let payload = encode_transaction(tx);
    let len = u32::try_from(payload.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "encoded transaction length exceeds u32",
        )
    })?;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(&len.to_be_bytes())?;
    f.write_all(&payload)?;
    f.sync_all()
}

/// Read all length-prefixed frames; on success replace file with empty.
fn drain_pending_txs(path: &Path) -> Result<Vec<Transaction>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read(path).map_err(|e| e.to_string())?;
    if data.is_empty() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        if pos + 4 > data.len() {
            return Err("pending_tx.tril: truncated length prefix".into());
        }
        let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if len as u32 > MAX_PENDING_TX_FRAME {
            return Err("pending_tx.tril: frame too large".into());
        }
        if pos + len > data.len() {
            return Err("pending_tx.tril: truncated frame body".into());
        }
        let tx = decode_transaction(&data[pos..pos + len]).map_err(|e| e.to_string())?;
        out.push(tx);
        pos += len;
    }

    fs::write(path, []).map_err(|e| e.to_string())?;
    Ok(out)
}

fn parse_run_args(
    args: &[String],
) -> Result<(PathBuf, u64, Option<String>, Vec<String>, Option<u64>), String> {
    let mut data_dir = PathBuf::from(".");
    let mut interval = DEFAULT_INTERVAL_SECS;
    let mut listen: Option<String> = None;
    let mut peers: Vec<String> = Vec::new();
    let mut max_future_drift_secs: Option<u64> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--data-dir needs a value".to_string())?;
                data_dir = PathBuf::from(v);
                i += 2;
            }
            "--interval-secs" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--interval-secs needs a value".to_string())?;
                interval = v
                    .parse()
                    .map_err(|_| "interval-secs must be a positive integer".to_string())?;
                if interval == 0 {
                    return Err("--interval-secs must be > 0".into());
                }
                i += 2;
            }
            "--listen" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--listen needs HOST:PORT".to_string())?;
                listen = Some(v.clone());
                i += 2;
            }
            "--peers" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--peers needs comma-separated HOST:PORT list".to_string())?;
                peers = v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                i += 2;
            }
            "--max-future-drift-secs" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--max-future-drift-secs needs a value".to_string())?;
                max_future_drift_secs = Some(
                    v.parse()
                        .map_err(|_| "max-future-drift-secs must be a u64".to_string())?,
                );
                i += 2;
            }
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    Ok((
        data_dir,
        interval,
        listen,
        peers,
        max_future_drift_secs,
    ))
}

/// Parses `--data-dir` and leaves remaining positional strings.
fn parse_dir_and_positionals(args: &[String]) -> Result<(PathBuf, Vec<String>), String> {
    let mut data_dir = PathBuf::from(".");
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--data-dir" {
            let v = args
                .get(i + 1)
                .ok_or_else(|| "--data-dir needs a value".to_string())?;
            data_dir = PathBuf::from(v);
            i += 2;
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    Ok((data_dir, rest))
}

fn cmd_init(data_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let path = data_dir.join(WALLET_FILE);
    if path.exists() {
        return Err(format!(
            "{} already exists; remove it first if you want a new wallet",
            path.display()
        ));
    }
    let w = Wallet::generate();
    fs::write(&path, w.seed_bytes()).map_err(|e| e.to_string())?;
    println!("Wallet written to {}", path.display());
    println!("Address: {}", w.address());
    eprintln!("Keep wallet.seed secret; it controls funds on this dev setup.");
    Ok(())
}

fn cmd_send(data_dir: &Path, receiver: &str, amount: u64, fee: u64) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let wallet = load_wallet(data_dir).map_err(|e| e.to_string())?;
    let chain_path = data_dir.join(CHAIN_FILE);
    let chain = load_blockchain_from_disk_with(&chain_path, |c| dev_bootstrap(&wallet, c))
        .map_err(|e| e.to_string())?;
    let nonce = chain
        .state()
        .get_account(&wallet.address())
        .map(|a| a.nonce)
        .unwrap_or(0);
    let ts = unix_now_secs();
    let tx = wallet
        .sign_transfer(Address::new(receiver), amount, fee, nonce, ts)
        .map_err(|e| e.to_string())?;
    let pending = data_dir.join(PENDING_TX_FILE);
    append_pending_tx(&pending, &tx).map_err(|e| e.to_string())?;
    println!(
        "Queued tx {} -> {} amount {} fee {} (nonce {})",
        tx.tx_hash, receiver, amount, fee, nonce
    );
    Ok(())
}

fn cmd_run(
    data_dir: &Path,
    interval_secs: u64,
    listen: Option<String>,
    peers: Vec<String>,
    max_future_drift_secs: Option<u64>,
) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let wallet = load_wallet(data_dir).map_err(|e| e.to_string())?;
    let chain_path = data_dir.join(CHAIN_FILE);
    let pending_path = data_dir.join(PENDING_TX_FILE);
    let mut chain = load_blockchain_from_disk_with(&chain_path, |c| dev_bootstrap(&wallet, c))
        .map_err(|e| e.to_string())?;
    if let Some(d) = max_future_drift_secs {
        chain.consensus_params_mut().max_future_drift_secs = d;
    }
    let store = BlockStore::open_append(&chain_path).map_err(|e| e.to_string())?;
    let inner = NodeInner {
        chain,
        pool: Mempool::new(10_000),
        store,
    };
    let state = Arc::new(Mutex::new(inner));

    for p in &peers {
        let now = unix_now_secs();
        let mut g = state.lock().expect("node lock");
        match node::network::sync_from_peer(&mut *g, p, now) {
            Ok(n) if n > 0 => eprintln!("sync: +{n} block(s) from {p}"),
            Ok(_) => {}
            Err(e) => eprintln!("sync from {p}: {e}"),
        }
    }

    if let Some(addr) = listen.clone() {
        match node::network::spawn_incoming_loop(&addr, state.clone()) {
            Ok((_h, bound)) => println!("network: listening on {bound}"),
            Err(e) => return Err(format!("network listen {addr}: {e}")),
        }
    }

    println!(
        "Trilogicon node | height={} | wallet={}",
        state.lock().expect("node lock").chain.height(),
        wallet.address()
    );
    println!("Ctrl+C to stop.");

    let peers_for_gossip = peers.clone();

    loop {
        thread::sleep(std::time::Duration::from_secs(interval_secs));

        let sealed = {
            let mut g = state.lock().expect("node lock");

            match drain_pending_txs(&pending_path) {
                Ok(txs) => {
                    for tx in txs {
                        match g.pool.try_submit(tx) {
                            Ok(()) => eprintln!("mempool: accepted tx"),
                            Err(e) => eprintln!("mempool: rejected ({e})"),
                        }
                    }
                }
                Err(e) => eprintln!("pending_tx.tril: {e}"),
            }

            let now = unix_now_secs();
            let NodeInner {
                chain,
                pool,
                store,
            } = &mut *g;
            match chain.append_block_from_mempool(pool, 64, now) {
                Ok(0) => None,
                Ok(n) => {
                    let tip = chain.blocks().last().expect("tip after append").clone();
                    if let Err(e) = store.append_block(&tip) {
                        eprintln!("persist block: {e}");
                        None
                    } else {
                        eprintln!("sealed height={} with {n} transaction(s)", chain.height());
                        Some(tip)
                    }
                }
                Err(e) => {
                    eprintln!("block production: {e}");
                    None
                }
            }
        };

        if let Some(ref block) = sealed {
            for p in &peers_for_gossip {
                if let Err(e) = node::network::push_block_to_peer(p, block) {
                    eprintln!("gossip block to {p}: {e}");
                }
            }
        }
    }
}

fn run_cli(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("missing command".into());
    }
    let bin = Path::new(&args[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("node");

    match args[1].as_str() {
        "init" => {
            let (dir, rest) = parse_dir_and_positionals(&args[2..])?;
            if !rest.is_empty() {
                return Err(format!("unexpected arguments: {}", rest.join(" ")));
            }
            cmd_init(&dir)
        }
        "run" => {
            let (dir, interval, listen, peers, max_future) = parse_run_args(&args[2..])?;
            cmd_run(&dir, interval, listen, peers, max_future)
        }
        "send" => {
            let (dir, rest) = parse_dir_and_positionals(&args[2..])?;
            if rest.len() < 2 {
                return Err("send: need RECEIVER AMOUNT [FEE]".into());
            }
            let receiver = rest[0].clone();
            let amount: u64 = rest[1]
                .parse()
                .map_err(|_| "amount must be a u64".to_string())?;
            let fee: u64 = if rest.len() > 2 {
                rest[2]
                    .parse()
                    .map_err(|_| "fee must be a u64".to_string())?
            } else {
                1
            };
            cmd_send(&dir, &receiver, amount, fee)
        }
        _ => {
            usage(bin);
            Err("unknown command".into())
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && matches!(args[1].as_str(), "-h" | "--help" | "help") {
        let bin = Path::new(&args[0])
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("node");
        usage(bin);
        return;
    }

    match run_cli(&args) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
