//! Dev CLI: wallet, protocol genesis, chain dir, mempool, optional P2P.
//!
//! ```text
//! cargo run -p node -- init [--data-dir DIR] [--genesis-balance N]
//! cargo run -p node -- run [--data-dir DIR] [--genesis PATH] [--interval-secs SECS]
//!   [--listen HOST:PORT] [--peers HOST:PORT,...] [--max-future-drift-secs N]
//! cargo run -p node -- send [--data-dir DIR] [--genesis PATH] RECEIVER AMOUNT [FEE]
//! ```
//!
//! Default genesis path: `{data-dir}/genesis.toml`. Use `init --genesis-balance` to create it.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use node::encoding::{decode_transaction, encode_transaction};
use node::genesis::{Genesis, GenesisAllocation};
use node::mempool::Mempool;
use node::network::{NodeInner, WireRuntimeConfig};
use node::peer_book::PeerBook;
use node::storage::{BlockStore, load_blockchain_from_disk};
use node::transaction::Transaction;
use node::types::Address;
use node::wallet::Wallet;

const WALLET_FILE: &str = "wallet.seed";
const CHAIN_FILE: &str = "chain.blocks";
const PENDING_TX_FILE: &str = "pending_tx.tril";
const GENESIS_FILE: &str = "genesis.toml";
const DEFAULT_INTERVAL_SECS: u64 = 2;
const MAX_PENDING_TX_FRAME: u32 = 4 * 1024 * 1024;
const DEFAULT_NETWORK_ID: u32 = 1;

fn usage(bin: &str) {
    eprintln!(
        "Usage:
  {bin} init [--data-dir DIR] [--genesis-balance N]
  {bin} run [--data-dir DIR] [--genesis PATH] [--interval-secs SECS]
          [--listen HOST:PORT] [--peers HOST:PORT,...] [--max-future-drift-secs N]
          [--network-id N] [--handshake] [--require-handshake-inbound] [--no-legacy-inbound]
          [--exchange-peers] [--announce-blocks]
  {bin} send [--data-dir DIR] [--genesis PATH] RECEIVER AMOUNT [FEE]

Genesis: default {GENESIS_FILE} under --data-dir. init --genesis-balance writes it for the new wallet.
Files: {WALLET_FILE}, {CHAIN_FILE}, {PENDING_TX_FILE}, {GENESIS_FILE}"
    );
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn resolve_genesis_path(data_dir: &Path, genesis_flag: Option<&str>) -> PathBuf {
    genesis_flag
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join(GENESIS_FILE))
}

fn load_genesis_for_cmd(data_dir: &Path, genesis_flag: Option<&str>) -> Result<Genesis, String> {
    let p = resolve_genesis_path(data_dir, genesis_flag);
    if !p.exists() {
        return Err(format!(
            "genesis file not found: {} (use `init --genesis-balance N` or --genesis PATH)",
            p.display()
        ));
    }
    Genesis::from_path(&p).map_err(|e| format!("{}: {e}", p.display()))
}

fn load_wallet(data_dir: &Path) -> std::io::Result<Wallet> {
    let path = data_dir.join(WALLET_FILE);
    let raw = fs::read(&path).map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            std::io::Error::new(
                ErrorKind::NotFound,
                format!(
                    "{}: no wallet here — run `init --data-dir {}` first",
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
            format!(
                "expected 32-byte {}, got {} bytes",
                path.display(),
                raw.len()
            ),
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
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(&len.to_be_bytes())?;
    f.write_all(&payload)?;
    f.sync_all()
}

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
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
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

#[allow(clippy::type_complexity)]
fn parse_run_args(
    args: &[String],
) -> Result<
    (
        PathBuf,
        Option<String>,
        u64,
        Option<String>,
        Vec<String>,
        Option<u64>,
        u32,
        bool,
        bool,
        bool,
        bool,
        bool,
    ),
    String,
> {
    let mut data_dir = PathBuf::from(".");
    let mut genesis_path: Option<String> = None;
    let mut interval = DEFAULT_INTERVAL_SECS;
    let mut listen: Option<String> = None;
    let mut peers: Vec<String> = Vec::new();
    let mut max_future_drift_secs: Option<u64> = None;
    let mut network_id = DEFAULT_NETWORK_ID;
    let mut handshake_outbound = false;
    let mut require_handshake_inbound = false;
    let mut allow_legacy_inbound = true;
    let mut exchange_peers = false;
    let mut announce_blocks = false;
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
            "--genesis" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--genesis needs a file path".to_string())?;
                genesis_path = Some(v.clone());
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
            "--network-id" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--network-id needs a value".to_string())?;
                network_id = v
                    .parse()
                    .map_err(|_| "--network-id must be a u32".to_string())?;
                i += 2;
            }
            "--handshake" => {
                handshake_outbound = true;
                i += 1;
            }
            "--require-handshake-inbound" => {
                require_handshake_inbound = true;
                i += 1;
            }
            "--no-legacy-inbound" => {
                allow_legacy_inbound = false;
                i += 1;
            }
            "--exchange-peers" => {
                exchange_peers = true;
                i += 1;
            }
            "--announce-blocks" => {
                announce_blocks = true;
                i += 1;
            }
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    Ok((
        data_dir,
        genesis_path,
        interval,
        listen,
        peers,
        max_future_drift_secs,
        network_id,
        handshake_outbound,
        require_handshake_inbound,
        allow_legacy_inbound,
        exchange_peers,
        announce_blocks,
    ))
}

fn parse_init_args(args: &[String]) -> Result<(PathBuf, Option<u64>), String> {
    let mut data_dir = PathBuf::from(".");
    let mut genesis_balance: Option<u64> = None;
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
            "--genesis-balance" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--genesis-balance needs a u64".to_string())?;
                genesis_balance = Some(
                    v.parse()
                        .map_err(|_| "--genesis-balance must be a u64".to_string())?,
                );
                if genesis_balance == Some(0) {
                    return Err("--genesis-balance must be > 0".into());
                }
                i += 2;
            }
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    Ok((data_dir, genesis_balance))
}

fn parse_send_args(args: &[String]) -> Result<(PathBuf, Option<String>, Vec<String>), String> {
    let mut data_dir = PathBuf::from(".");
    let mut genesis_path: Option<String> = None;
    let mut rest = Vec::new();
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
            "--genesis" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--genesis needs a file path".to_string())?;
                genesis_path = Some(v.clone());
                i += 2;
            }
            _ => {
                rest.push(args[i].clone());
                i += 1;
            }
        }
    }
    Ok((data_dir, genesis_path, rest))
}

fn cmd_init(data_dir: &Path, genesis_balance: Option<u64>) -> Result<(), String> {
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

    if let Some(bal) = genesis_balance {
        let gen_path = data_dir.join(GENESIS_FILE);
        if gen_path.exists() {
            return Err(format!(
                "{} already exists; remove it first or omit --genesis-balance",
                gen_path.display()
            ));
        }
        let g = Genesis {
            allocations: vec![GenesisAllocation {
                address: w.address().0.clone(),
                balance: bal,
            }],
        };
        g.write_to_path(&gen_path)?;
        let hex = g.state_commitment_hex().map_err(|e| e.to_string())?;
        println!("Genesis written to {}", gen_path.display());
        println!("Genesis state commitment: {hex}");
    }

    eprintln!("Keep wallet.seed secret.");
    Ok(())
}

fn cmd_send(
    data_dir: &Path,
    genesis_flag: Option<&str>,
    receiver: &str,
    amount: u64,
    fee: u64,
) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let wallet = load_wallet(data_dir).map_err(|e| e.to_string())?;
    let genesis = load_genesis_for_cmd(data_dir, genesis_flag)?;
    let chain_path = data_dir.join(CHAIN_FILE);
    let (chain, repaired) =
        load_blockchain_from_disk(&chain_path, &genesis).map_err(|e| e.to_string())?;
    if repaired {
        node::diag::line(
            "storage",
            "repaired chain.blocks (truncated tail removed; see last complete block)",
        );
    }
    let nonce = chain
        .state()
        .get_account(&wallet.address())
        .ok_or_else(|| {
            "wallet address has no genesis allocation; add it to genesis.toml (same commitment as peers)".to_string()
        })?
        .nonce;
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

#[allow(clippy::too_many_arguments)]
fn cmd_run(
    data_dir: &Path,
    genesis_flag: Option<&str>,
    interval_secs: u64,
    listen: Option<String>,
    peers: Vec<String>,
    max_future_drift_secs: Option<u64>,
    network_id: u32,
    handshake_outbound: bool,
    require_handshake_inbound: bool,
    allow_legacy_inbound: bool,
    exchange_peers: bool,
    announce_blocks: bool,
) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let wallet = load_wallet(data_dir).map_err(|e| e.to_string())?;
    let genesis = load_genesis_for_cmd(data_dir, genesis_flag)?;
    let chain_path = data_dir.join(CHAIN_FILE);
    let pending_path = data_dir.join(PENDING_TX_FILE);
    let (mut chain, repaired) =
        load_blockchain_from_disk(&chain_path, &genesis).map_err(|e| e.to_string())?;
    if repaired {
        node::diag::line(
            "storage",
            "repaired chain.blocks (truncated tail removed; see last complete block)",
        );
    }
    if let Some(d) = max_future_drift_secs {
        chain.consensus_params_mut().max_future_drift_secs = d;
    }
    let store = BlockStore::open_append(&chain_path).map_err(|e| e.to_string())?;
    let wire = WireRuntimeConfig::from_genesis(&genesis, network_id, handshake_outbound)?
        .with_inbound_policy(require_handshake_inbound, allow_legacy_inbound)
        .with_gossip_extensions(exchange_peers, announce_blocks);

    let mut peer_book = PeerBook::load_or_empty(&PeerBook::path_in_data_dir(data_dir))
        .map_err(|e| e.to_string())?;
    for p in &peers {
        peer_book.merge_seed(p.clone());
    }
    peer_book
        .save(&PeerBook::path_in_data_dir(data_dir))
        .map_err(|e| e.to_string())?;

    let inner = NodeInner {
        genesis: genesis.clone(),
        wire,
        chain,
        pool: Mempool::new(10_000),
        store,
        seen_tx: node::seen::SeenCache::new(50_000),
        seen_block: node::seen::SeenCache::new(50_000),
        peer_book,
    };
    let state = Arc::new(Mutex::new(inner));

    let sync_targets = {
        let g = state.lock().expect("node lock");
        g.peer_book.sync_candidates(&peers)
    };
    if !sync_targets.is_empty() {
        let now = unix_now_secs();
        let filtered = {
            let g = state.lock().expect("node lock");
            g.peer_book.filter_available(&sync_targets, now)
        };
        let mut g = state.lock().expect("node lock");
        for p in &filtered {
            match node::network::sync_from_peer(&mut g, p, now) {
                Ok(n) if n > 0 => {
                    node::diag::line("sync", format!("+{n} block(s) from {p}"));
                    g.peer_book.record_ok(p);
                }
                Ok(_) => g.peer_book.record_ok(p),
                Err(e) if e.starts_with(node::network::FATAL_SYNC_PREFIX) => {
                    drop(g);
                    return Err(e);
                }
                Err(_) => g.peer_book.record_fail(p),
            }
        }
        g.peer_book
            .save(&PeerBook::path_in_data_dir(data_dir))
            .map_err(|e| e.to_string())?;
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
                            Ok(()) => node::diag::line("mempool", "accepted tx"),
                            Err(e) => node::diag::line("mempool", format!("rejected ({e})")),
                        }
                    }
                }
                Err(e) => node::diag::line("pending_tx", e),
            }

            let now = unix_now_secs();
            let sealed_block = {
                let inner = &mut *g;
                match inner
                    .chain
                    .append_block_from_mempool(&mut inner.pool, 64, now)
                {
                    Ok(0) => None,
                    Ok(n) => {
                        let tip = inner
                            .chain
                            .blocks()
                            .last()
                            .expect("tip after append")
                            .clone();
                        let durable_height = tip.height.saturating_sub(1);
                        if let Err(e) = inner.store.append_block(&tip) {
                            node::diag::line(
                                "persist",
                                format!(
                                    "seal write failed ({e}); rollback to height {durable_height}"
                                ),
                            );
                            if let Err(rerr) = inner
                                .chain
                                .rollback_to_height(durable_height, &inner.genesis)
                            {
                                return Err(format!(
                                    "{}persist/rollback failure: {rerr}; inspect {}",
                                    node::network::FATAL_SYNC_PREFIX,
                                    chain_path.display()
                                ));
                            }
                            for tx in &tip.transactions {
                                if let Err(me) = inner.pool.try_submit(tx.clone()) {
                                    node::diag::line(
                                        "mempool",
                                        format!("re-queue after rollback failed ({me})"),
                                    );
                                }
                            }
                            None
                        } else {
                            node::diag::line(
                                "produce",
                                format!("sealed height={} txs={n}", inner.chain.height()),
                            );
                            Some(tip)
                        }
                    }
                    Err(e) => {
                        node::diag::line("produce", format!("not sealed ({e})"));
                        None
                    }
                }
            };

            if !peers_for_gossip.is_empty() {
                let now_sync = unix_now_secs();
                for p in &peers_for_gossip {
                    g.peer_book.merge_seed(p.clone());
                }
                let targets = g.peer_book.sync_candidates(&peers_for_gossip);
                let filtered = g.peer_book.filter_available(&targets, now_sync);
                for p in &filtered {
                    match node::network::sync_from_peer(&mut g, p, now_sync) {
                        Ok(n) if n > 0 => {
                            node::diag::line("sync", format!("+{n} block(s) from {p}"));
                            g.peer_book.record_ok(p);
                        }
                        Ok(_) => g.peer_book.record_ok(p),
                        Err(e) if e.starts_with(node::network::FATAL_SYNC_PREFIX) => {
                            drop(g);
                            return Err(e);
                        }
                        Err(_) => g.peer_book.record_fail(p),
                    }
                }
                g.peer_book
                    .save(&PeerBook::path_in_data_dir(data_dir))
                    .map_err(|e| e.to_string())?;
            }

            sealed_block
        };

        if let Some(ref block) = sealed {
            let wire_snap = {
                let g = state.lock().expect("node lock");
                g.wire.clone()
            };
            for p in &peers_for_gossip {
                if let Err(e) = node::network::push_block_to_peer_inner(
                    p,
                    block,
                    Some(&wire_snap),
                    block.height,
                    &block.block_hash,
                ) {
                    node::diag::line("gossip", format!("[{p}] push block failed: {e}"));
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
            let (dir, genesis_bal) = parse_init_args(&args[2..])?;
            cmd_init(&dir, genesis_bal)
        }
        "run" => {
            let (
                dir,
                genesis,
                interval,
                listen,
                peers,
                max_future,
                network_id,
                handshake_out,
                require_hs_in,
                allow_legacy_in,
                exchange_peers,
                announce_blocks,
            ) = parse_run_args(&args[2..])?;
            cmd_run(
                &dir,
                genesis.as_deref(),
                interval,
                listen,
                peers,
                max_future,
                network_id,
                handshake_out,
                require_hs_in,
                allow_legacy_in,
                exchange_peers,
                announce_blocks,
            )
        }
        "send" => {
            let (dir, genesis, rest) = parse_send_args(&args[2..])?;
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
            cmd_send(&dir, genesis.as_deref(), &receiver, amount, fee)
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
