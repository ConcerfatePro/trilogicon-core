//! Dev CLI: wallet, protocol genesis, chain dir, mempool, optional P2P.
//!
//! ```text
//! cargo run -p node -- init [--data-dir DIR] [--genesis-balance N]
//! cargo run -p node -- run [--data-dir DIR] [--genesis PATH] [--interval-secs SECS]
//!   [--listen HOST:PORT] [--peers HOST:PORT,...] [--max-future-drift-secs N]
//!   [--max-inbound-peers N] [--peer-idle-timeout-secs N] [--peer-write-timeout-secs N]
//!   [--peer-max-wire-errors N] [--peer-max-frames N]
//!   [--peer-max-stale-blocks N] [--peer-max-inbound-tx N] [--mempool-capacity N]
//! cargo run -p node -- send [--data-dir DIR] [--genesis PATH] RECEIVER AMOUNT [FEE]
//! ```
//!
//! Default genesis path: `{data-dir}/genesis.toml`. Use `init --genesis-balance` to create it.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use node::data_dir_bind::{self, GENESIS_BIND_LOCK_FILE, NODE_RUN_LOCK_FILE};
use node::file_lock::ExclusiveFileLock;
use node::genesis::{Genesis, GenesisAllocation};
use node::mempool::Mempool;
use node::network::{InboundPeerPolicy, NodeInner, SyncWorkBudget};
use node::operator_msg::{PFX_MEMPOOL, PFX_PENDING, PFX_PEER, PFX_SEAL, PFX_STARTUP, PFX_STORAGE, PFX_SYNC};
use node::pending_tx_file::{
    append_pending_transaction, drain_pending_file, PENDING_TX_LOCK_FILE,
};
use node::peer_book::PeerBook;
use node::storage::{BlockStore, load_blockchain_from_disk};
use node::types::Address;
use node::wallet::Wallet;

const WALLET_FILE: &str = "wallet.seed";
const CHAIN_FILE: &str = "chain.blocks";
const PENDING_TX_FILE: &str = "pending_tx.tril";
const GENESIS_FILE: &str = "genesis.toml";
const DEFAULT_INTERVAL_SECS: u64 = 2;
const DEFAULT_MEMPOOL_CAPACITY: usize = 10_000;
const MAX_MEMPOOL_CAPACITY: usize = 1_000_000;

fn usage(bin: &str) {
    eprintln!(
        "Usage:
  {bin} init [--data-dir DIR] [--genesis-balance N]
  {bin} run [--data-dir DIR] [--genesis PATH] [--interval-secs SECS]
          [--listen HOST:PORT] [--peers HOST:PORT,...] [--max-future-drift-secs N]
          [--max-inbound-peers N] [--peer-idle-timeout-secs N] [--peer-write-timeout-secs N]
          [--peer-max-wire-errors N] [--peer-max-frames N]
          [--peer-max-stale-blocks N] [--peer-max-inbound-tx N] [--mempool-capacity N]
  {bin} send [--data-dir DIR] [--genesis PATH] RECEIVER AMOUNT [FEE]

Genesis: default {GENESIS_FILE} under --data-dir. init --genesis-balance writes it for the new wallet.
Files: {WALLET_FILE}, {CHAIN_FILE}, {PENDING_TX_FILE} (lock: {PENDING_LOCK}), {GENESIS_FILE}, {BIND_FILE} (lock: {GENESIS_LOCK}), {RUN_LOCK} (exclusive `run` lock)",
        BIND_FILE = data_dir_bind::GENESIS_BIND_FILE,
        PENDING_LOCK = PENDING_TX_LOCK_FILE,
        GENESIS_LOCK = GENESIS_BIND_LOCK_FILE,
        RUN_LOCK = NODE_RUN_LOCK_FILE
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
        InboundPeerPolicy,
        usize,
    ),
    String,
> {
    let mut data_dir = PathBuf::from(".");
    let mut genesis_path: Option<String> = None;
    let mut interval = DEFAULT_INTERVAL_SECS;
    let mut listen: Option<String> = None;
    let mut peers: Vec<String> = Vec::new();
    let mut max_future_drift_secs: Option<u64> = None;
    let mut mempool_capacity: usize = DEFAULT_MEMPOOL_CAPACITY;
    let mut peer_policy = InboundPeerPolicy::default();
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
            "--max-inbound-peers" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--max-inbound-peers needs a value".to_string())?;
                let n: u64 = v
                    .parse()
                    .map_err(|_| "--max-inbound-peers must be a non-negative integer".to_string())?;
                peer_policy.max_concurrent_sessions =
                    usize::try_from(n).map_err(|_| "--max-inbound-peers too large".to_string())?;
                i += 2;
            }
            "--peer-idle-timeout-secs" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--peer-idle-timeout-secs needs a value".to_string())?;
                let n: u64 = v
                    .parse()
                    .map_err(|_| "--peer-idle-timeout-secs must be a positive integer".to_string())?;
                if n == 0 {
                    return Err("--peer-idle-timeout-secs must be >= 1".into());
                }
                peer_policy.idle_read_timeout = Duration::from_secs(n);
                i += 2;
            }
            "--peer-write-timeout-secs" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--peer-write-timeout-secs needs a value".to_string())?;
                let n: u64 = v.parse().map_err(|_| {
                    "--peer-write-timeout-secs must be a non-negative integer".to_string()
                })?;
                peer_policy.write_timeout = if n == 0 {
                    None
                } else {
                    Some(Duration::from_secs(n))
                };
                i += 2;
            }
            "--peer-max-wire-errors" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--peer-max-wire-errors needs a value".to_string())?;
                let n: u32 = v
                    .parse()
                    .map_err(|_| "--peer-max-wire-errors must be a u32 >= 1".to_string())?;
                if n < 1 {
                    return Err("--peer-max-wire-errors must be >= 1".into());
                }
                peer_policy.max_protocol_errors_per_session = n;
                i += 2;
            }
            "--peer-max-frames" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--peer-max-frames needs a value".to_string())?;
                let n: u32 = v
                    .parse()
                    .map_err(|_| "--peer-max-frames must be a u32 >= 1".to_string())?;
                if n < 1 {
                    return Err("--peer-max-frames must be >= 1".into());
                }
                peer_policy.max_app_frames_per_session = n;
                i += 2;
            }
            "--peer-max-stale-blocks" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--peer-max-stale-blocks needs a value".to_string())?;
                let n: u32 = v
                    .parse()
                    .map_err(|_| "--peer-max-stale-blocks must be a u32 >= 1".to_string())?;
                if n < 1 {
                    return Err("--peer-max-stale-blocks must be >= 1".into());
                }
                peer_policy.max_stale_decoded_blocks_per_session = n;
                i += 2;
            }
            "--peer-max-inbound-tx" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--peer-max-inbound-tx needs a value".to_string())?;
                let n: u32 = v
                    .parse()
                    .map_err(|_| "--peer-max-inbound-tx must be a u32 >= 1".to_string())?;
                if n < 1 {
                    return Err("--peer-max-inbound-tx must be >= 1".into());
                }
                peer_policy.max_inbound_tx_per_session = n;
                i += 2;
            }
            "--mempool-capacity" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--mempool-capacity needs a value".to_string())?;
                let n: usize = v
                    .parse()
                    .map_err(|_| "--mempool-capacity must be a usize >= 1".to_string())?;
                if n < 1 {
                    return Err("--mempool-capacity must be >= 1".into());
                }
                if n > MAX_MEMPOOL_CAPACITY {
                    return Err(format!(
                        "--mempool-capacity must be <= {MAX_MEMPOOL_CAPACITY} (local RAM bound)"
                    ));
                }
                mempool_capacity = n;
                i += 2;
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
        peer_policy,
        mempool_capacity,
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
    data_dir_bind::verify_binding_if_present(data_dir, &genesis)?;
    let chain_path = data_dir.join(CHAIN_FILE);
    let (chain, _repaired) = load_blockchain_from_disk(&chain_path, &genesis).map_err(|e| {
        format!(
            "{PFX_STARTUP} fail-closed: could not load chain {} — {e}. Repair or replace chain.blocks if corrupt or truncated. See docs/design_notes/v2_persistence_restart.md",
            chain_path.display()
        )
    })?;
    data_dir_bind::ensure_binding_if_missing(data_dir, &genesis)?;
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
    append_pending_transaction(&pending, &tx).map_err(|e| e.to_string())?;
    println!(
        "Queued tx {} -> {} amount {} fee {} (nonce {})",
        tx.tx_hash, receiver, amount, fee, nonce
    );
    Ok(())
}

const GOSSIP_FAIL_THRESHOLD: u32 = 5;
const GOSSIP_COOLDOWN_SECS: u64 = 45;

fn cmd_run(
    data_dir: &Path,
    genesis_flag: Option<&str>,
    interval_secs: u64,
    listen: Option<String>,
    peers: Vec<String>,
    max_future_drift_secs: Option<u64>,
    inbound_peer_policy: InboundPeerPolicy,
    mempool_capacity: usize,
) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;

    let run_lock_path = data_dir.join(NODE_RUN_LOCK_FILE);
    let _run_dir_lock = match ExclusiveFileLock::try_acquire_exclusive(&run_lock_path) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            return Err(format!(
                "{PFX_STARTUP} fail-closed: another `node run` already holds this data directory (lock file {}). Stop the other process or use a different --data-dir.",
                run_lock_path.display()
            ));
        }
        Err(e) => {
            return Err(format!(
                "{PFX_STARTUP} fail-closed: could not acquire run lock {} — {e}",
                run_lock_path.display()
            ));
        }
    };

    let wallet = load_wallet(data_dir).map_err(|e| e.to_string())?;
    let genesis = load_genesis_for_cmd(data_dir, genesis_flag)?;
    data_dir_bind::verify_binding_if_present(data_dir, &genesis)?;
    let chain_path = data_dir.join(CHAIN_FILE);
    let pending_path = data_dir.join(PENDING_TX_FILE);
    let (mut chain, _repaired) = load_blockchain_from_disk(&chain_path, &genesis).map_err(|e| {
        format!(
            "{PFX_STARTUP} fail-closed: could not load chain {} — {e}. Repair or replace chain.blocks if corrupt or truncated. See docs/design_notes/v2_persistence_restart.md",
            chain_path.display()
        )
    })?;
    data_dir_bind::ensure_binding_if_missing(data_dir, &genesis)?;
    if let Some(d) = max_future_drift_secs {
        chain.consensus_params_mut().max_future_drift_secs = d;
    }
    let store = BlockStore::open_append(&chain_path).map_err(|e| e.to_string())?;
    let mut peer_book = PeerBook::load_or_empty(&PeerBook::path_in_data_dir(data_dir))
        .map_err(|e| e.to_string())?;
    for p in &peers {
        peer_book.merge_seed(p.clone());
    }
    peer_book
        .save(&PeerBook::path_in_data_dir(data_dir))
        .map_err(|e| e.to_string())?;
    eprintln!(
        "{PFX_STARTUP} mempool capacity {mempool_capacity} tx slots (local bound; not consensus)"
    );
    let inner = NodeInner {
        genesis: genesis.clone(),
        chain,
        pool: Mempool::new(mempool_capacity),
        store,
        seen_tx: node::seen::SeenCache::new(50_000),
        seen_block: node::seen::SeenCache::new(50_000),
        peer_book,
    };
    let state = Arc::new(Mutex::new(inner));

    let sync_budget = SyncWorkBudget::default();
    let mut sealing_allowed = peers.is_empty();
    if !peers.is_empty() {
        let mut g = state.lock().expect("node lock");
        let mut all_ok = true;
        for p in &peers {
            match node::network::sync_from_peer(&mut g, p, &sync_budget) {
                Ok(out) => {
                    if out.blocks_appended > 0 {
                        eprintln!(
                            "{PFX_SYNC} +{} block(s) appended from {p}",
                            out.blocks_appended
                        );
                    }
                    if out.stopped_due_to_budget {
                        eprintln!(
                            "{PFX_SYNC} bounded stop: local work budget reached for {p} (stopped_due_to_budget) — next sync resumes from current height; operational cap only, not consensus"
                        );
                    }
                }
                Err(e) => {
                    all_ok = false;
                    eprintln!("{PFX_SYNC} catch-up from {p} failed: {e}");
                }
            }
        }
        sealing_allowed = all_ok;
        if !sealing_allowed {
            eprintln!(
                "{PFX_SYNC} initial catch-up: not all --peers succeeded — local block sealing is disabled until every configured peer responds successfully (retried each interval). This node has no fork-choice repair: sealing while peers are configured but unreachable risks an isolated stale tip. Fix peers, drop --peers for a solo producer, or wait until catch-up succeeds."
            );
        }
        g.peer_book
            .save(&PeerBook::path_in_data_dir(data_dir))
            .map_err(|e| e.to_string())?;
    }

    if let Some(addr) = listen.clone() {
        match node::network::spawn_incoming_loop(&addr, state.clone(), inbound_peer_policy) {
            Ok((_h, bound)) => println!("{PFX_PEER} listening on {bound}"),
            Err(e) => return Err(format!("network listen {addr}: {e}")),
        }
    }

    println!(
        "{PFX_STARTUP} Trilogicon node | height={} | wallet={}",
        state.lock().expect("node lock").chain.height(),
        wallet.address()
    );
    println!("{PFX_STARTUP} Ctrl+C to stop.");

    let peers_for_gossip = peers.clone();
    let gossip_genesis = genesis.clone();
    let mut gossip_cooldown_until: HashMap<String, u64> = HashMap::new();
    let mut gossip_fail_streak: HashMap<String, u32> = HashMap::new();

    loop {
        thread::sleep(std::time::Duration::from_secs(interval_secs));

        let sealed = {
            let mut g = state.lock().expect("node lock");

            if g.store.is_poisoned() {
                return Err(format!(
                    "{PFX_STORAGE} fail-closed: store poisoned in this process after a prior append/sync failure — stop; repair or restore chain.blocks before restart"
                ));
            }

            if let Err(e) = drain_pending_file(&pending_path, &mut g.pool) {
                eprintln!(
                    "{PFX_PENDING} drain failed: {e} — not necessarily parse; lock/read/encode/rewrite failures have different implications (docs/design_notes/v2_persistence_restart.md)"
                );
            }

            if !sealing_allowed && !peers_for_gossip.is_empty() {
                let mut all_ok = true;
                for p in &peers_for_gossip {
                    match node::network::sync_from_peer(&mut g, p, &sync_budget) {
                        Ok(out) => {
                            if out.blocks_appended > 0 {
                                eprintln!(
                                    "{PFX_SYNC} +{} block(s) appended from {p}",
                                    out.blocks_appended
                                );
                            }
                            if out.stopped_due_to_budget {
                                eprintln!(
                                    "{PFX_SYNC} bounded stop: local work budget reached for {p} (stopped_due_to_budget) — next sync resumes from current height; operational cap only, not consensus"
                                );
                            }
                        }
                        Err(e) => {
                            all_ok = false;
                            eprintln!("{PFX_SYNC} catch-up from {p} failed: {e}");
                        }
                    }
                }
                if all_ok {
                    sealing_allowed = true;
                    eprintln!(
                        "{PFX_SYNC} all configured peers completed catch-up — local sealing enabled"
                    );
                }
            }

            let now = unix_now_secs();
            let committed = g.chain.state().clone();
            let (fifo, stale, dup) = g.pool.hygiene_vs_committed_ledger(&committed);
            if fifo > 0 || stale > 0 || dup > 0 {
                eprintln!(
                    "{PFX_MEMPOOL} hygiene vs committed ledger: FIFO-cleaned {fifo}, stale-nonce dropped {stale}, sender+nonce dup dropped {dup} (local policy)"
                );
            }
            let seal_result = if sealing_allowed {
                let NodeInner {
                    chain,
                    pool,
                    ..
                } = &mut *g;
                chain.append_block_from_mempool_pending_removal(pool, 64, now)
            } else {
                Ok(None)
            };
            let sealed_block = match seal_result {
                Ok(None) => None,
                Ok(Some(hashes)) => {
                    let n = hashes.len();
                    let tip = g.chain.blocks().last().expect("tip after append").clone();
                    match g.store.append_block(&tip) {
                        Ok(()) => {
                            g.pool
                                .remove_by_tx_hashes(hashes.iter().map(|s| s.as_str()));
                            eprintln!(
                                "{PFX_SEAL} committed height={} with {n} transaction(s)",
                                g.chain.height()
                            );
                            Some(tip)
                        }
                        Err(e) => {
                            let genesis = g.genesis.clone();
                            if let Err(r) = g.chain.rollback_last_block(&genesis) {
                                eprintln!(
                                    "{PFX_STORAGE} FATAL: persist block failed ({e}); in-memory rollback failed ({r}) — disk and memory may disagree; stop and repair chain.blocks"
                                );
                                return Err(
                                    "chain.blocks persist failed and rollback failed".into(),
                                );
                            }
                            eprintln!(
                                "{PFX_STORAGE} fail-closed: persist block failed ({e}) — chain.blocks append/sync may be partial; stopping. Repair or restore chain.blocks before restart."
                            );
                            return Err(format!(
                                "{PFX_STORAGE} fail-closed: chain.blocks write failure — {e}"
                            ));
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "{PFX_SEAL} seal attempt failed (no block committed; mempool consistent with pre-seal rules): {e}"
                    );
                    let committed = g.chain.state().clone();
                    let (fifo, stale, dup) = g.pool.hygiene_vs_committed_ledger(&committed);
                    if fifo > 0 || stale > 0 || dup > 0 {
                        eprintln!(
                            "{PFX_MEMPOOL} after failed seal: FIFO-cleaned {fifo}, stale-nonce dropped {stale}, sender+nonce dup dropped {dup} (local policy)"
                        );
                    }
                    None
                }
            };

            let now_sync = unix_now_secs();
            if !peers_for_gossip.is_empty() {
                let targets = g.peer_book.sync_candidates(&peers_for_gossip);
                let filtered = g.peer_book.filter_available(&targets, now_sync);
                for p in &filtered {
                    match node::network::sync_from_peer(&mut g, p, &sync_budget) {
                        Ok(out) => {
                            if out.blocks_appended > 0 {
                                node::diag::line(
                                    "sync",
                                    format!("+{} block(s) from {p}", out.blocks_appended),
                                );
                            }
                            g.peer_book.record_ok(p);
                        }
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
            let adv = block.height;
            let now_g = unix_now_secs();
            for p in &peers_for_gossip {
                if gossip_cooldown_until
                    .get(p)
                    .is_some_and(|&t| t > now_g)
                {
                    continue;
                }
                match node::network::push_block_to_peer(p, &gossip_genesis, adv, block) {
                    Ok(()) => {
                        gossip_fail_streak.remove(p);
                    }
                    Err(e) => {
                        eprintln!("{PFX_PEER} gossip to {p} failed: {e} — peer may be down (not local corruption)");
                        let streak = gossip_fail_streak.entry(p.clone()).or_insert(0);
                        *streak = streak.saturating_add(1);
                        if *streak >= GOSSIP_FAIL_THRESHOLD {
                            gossip_cooldown_until.insert(p.clone(), now_g + GOSSIP_COOLDOWN_SECS);
                            gossip_fail_streak.insert(p.clone(), 0);
                            eprintln!(
                                "{PFX_PEER} gossip to {p}: {GOSSIP_FAIL_THRESHOLD} consecutive failures — {GOSSIP_COOLDOWN_SECS}s cooldown (local policy)"
                            );
                        }
                    }
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
            let (dir, genesis, interval, listen, peers, max_future, peer_policy, mempool_cap) =
                parse_run_args(&args[2..])?;
            cmd_run(
                &dir,
                genesis.as_deref(),
                interval,
                listen,
                peers,
                max_future,
                peer_policy,
                mempool_cap,
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

#[cfg(test)]
mod run_args_tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn mempool_capacity_defaults() {
        let (_, _, _, _, _, _, _, cap) = parse_run_args(&args(&[])).unwrap();
        assert_eq!(cap, DEFAULT_MEMPOOL_CAPACITY);
    }

    #[test]
    fn mempool_capacity_parses() {
        let (_, _, _, _, _, _, _, cap) =
            parse_run_args(&args(&["--mempool-capacity", "4096"])).unwrap();
        assert_eq!(cap, 4096);
    }

    #[test]
    fn mempool_capacity_rejects_zero() {
        assert!(parse_run_args(&args(&["--mempool-capacity", "0"])).is_err());
    }

    #[test]
    fn mempool_capacity_rejects_above_max() {
        let over = format!("{}", MAX_MEMPOOL_CAPACITY + 1);
        assert!(parse_run_args(&args(&["--mempool-capacity", &over])).is_err());
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
