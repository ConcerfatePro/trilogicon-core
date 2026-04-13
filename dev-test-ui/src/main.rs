//! **LOCAL TESTING ONLY** — bind `127.0.0.1`, read `chain.blocks` without repair, queue txs like `node send`.
//!
//! Run from repo root (example):
//! ```text
//! cd dev-test-ui && cargo run -- --data-dir ../node/data-a
//! ```
//! Then open http://127.0.0.1:9847 — keep `node run` in another terminal for sealing and sync.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use node::blockchain::Blockchain;
use node::data_dir_bind::{self, NODE_RUN_LOCK_FILE};
use node::file_lock::ExclusiveFileLock;
use node::genesis::Genesis;
use node::pending_tx_file::{append_pending_transaction, parse_pending_file_bytes};
use node::storage::BlockStore;
use node::types::Address;
use node::wallet::Wallet;
use serde::Serialize;

const CHAIN_FILE: &str = "chain.blocks";
const PENDING_TX_FILE: &str = "pending_tx.tril";
const GENESIS_FILE: &str = "genesis.toml";
const WALLET_FILE: &str = "wallet.seed";

#[derive(Clone)]
struct AppState {
    data_dir: PathBuf,
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn resolve_genesis_path(data_dir: &Path) -> PathBuf {
    data_dir.join(GENESIS_FILE)
}

fn load_genesis(data_dir: &Path) -> Result<Genesis, String> {
    let p = resolve_genesis_path(data_dir);
    if !p.exists() {
        return Err(format!(
            "genesis missing: {} (run `node init --data-dir ...` first)",
            p.display()
        ));
    }
    Genesis::from_path(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Read-only chain replay (no `read_all_blocks_repairing_tail` — avoids mutating disk from this tool).
fn load_chain_readonly(data_dir: &Path, genesis: &Genesis) -> Result<Blockchain, String> {
    let chain_path = data_dir.join(CHAIN_FILE);
    let blocks = BlockStore::read_all_blocks(&chain_path).map_err(|e| e.to_string())?;
    let mut chain = Blockchain::from_genesis(genesis).map_err(|e| e.to_string())?;
    for b in blocks {
        chain.append_block(b).map_err(|e| e.to_string())?;
    }
    Ok(chain)
}

fn load_wallet(data_dir: &Path) -> Result<Wallet, String> {
    let path = data_dir.join(WALLET_FILE);
    let raw = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if raw.len() != 32 {
        return Err(format!("{}: expected 32-byte seed", path.display()));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&raw);
    Ok(Wallet::from_seed(&seed))
}

#[derive(Serialize)]
struct OverviewOut {
    warning: &'static str,
    data_dir: String,
    height: u64,
    tip_hash: String,
    chain_error: Option<String>,
    pending_tx_count: Option<usize>,
    pending_error: Option<String>,
    /// `true` if another process likely holds `node run` lock.
    node_run_active: bool,
    mempool_note: &'static str,
    wallet_address: Option<String>,
    peer_seed_addrs: Vec<String>,
    blocks: Vec<BlockSummary>,
}

#[derive(Serialize)]
struct BlockSummary {
    height: u64,
    block_hash: String,
    tx_count: usize,
    timestamp_unix: u64,
}

#[derive(Serialize)]
struct TxSummary {
    tx_hash: String,
    sender: String,
    receiver: String,
    amount: u64,
    fee: u64,
    nonce: u64,
}

#[derive(Serialize)]
struct ActivityOut {
    warning: &'static str,
    blocks: Vec<BlockSummary>,
    recent_txs: Vec<TxSummary>,
    error: Option<String>,
}

#[derive(Serialize)]
struct AccountOut {
    address: String,
    found: bool,
    balance: Option<u64>,
    nonce: Option<u64>,
    error: Option<String>,
}

#[derive(Serialize)]
struct WalletOut {
    address: Option<String>,
    /// Committed ledger balance for `wallet.seed` (next seal expects this for fee math).
    balance: Option<u64>,
    /// Next nonce the chain expects for this sender (`apply_transaction` uses this).
    committed_next_nonce: Option<u64>,
    error: Option<String>,
}

#[derive(Serialize)]
struct SetupOut {
    warning: &'static str,
    data_dir: String,
    paths: PathsOut,
    example_commands: Vec<String>,
}

#[derive(Serialize)]
struct PathsOut {
    genesis: String,
    chain_blocks: String,
    pending_tx: String,
    wallet_seed: String,
    genesis_bind: String,
    run_lock: String,
    peer_book: String,
}

#[derive(serde::Deserialize)]
struct SendBody {
    receiver: String,
    amount: u64,
    fee: u64,
    /// If set, sign with this nonce instead of the committed next nonce (stale / gap testing).
    #[serde(default)]
    nonce: Option<u64>,
}

#[derive(serde::Deserialize)]
struct AccountQuery {
    addr: String,
}

async fn get_overview(State(st): State<AppState>) -> impl IntoResponse {
    let data_dir = st.data_dir.clone();
    let res = tokio::task::spawn_blocking(move || overview_inner(&data_dir)).await;
    match res {
        Ok(Ok(json)) => (StatusCode::OK, Json(json)).into_response(),
        Ok(Err(e)) => (
            StatusCode::OK,
            Json(OverviewOut {
                warning: WARN,
                data_dir: st.data_dir.display().to_string(),
                height: 0,
                tip_hash: String::new(),
                chain_error: Some(e),
                pending_tx_count: None,
                pending_error: None,
                node_run_active: false,
                mempool_note: MEMPOOL_NOTE,
                wallet_address: None,
                peer_seed_addrs: vec![],
                blocks: vec![],
            }),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "task join failed"})),
        )
            .into_response(),
    }
}

const WARN: &str = "LOCAL TESTING ONLY — not a wallet; not for production; localhost binding only.";
const MEMPOOL_NOTE: &str = "Mempool lives inside a running `node run` process only; this UI reads disk (chain + pending file) and cannot show live mempool size.";

fn overview_inner(data_dir: &Path) -> Result<OverviewOut, String> {
    let genesis = load_genesis(data_dir)?;
    let run_lock_path = data_dir.join(NODE_RUN_LOCK_FILE);
    let node_run_active = matches!(
        ExclusiveFileLock::try_acquire_exclusive(&run_lock_path),
        Ok(None)
    );

    let (chain_result, chain_error) = match load_chain_readonly(data_dir, &genesis) {
        Ok(c) => (Some(c), None),
        Err(e) => (None, Some(e)),
    };

    let pending_path = data_dir.join(PENDING_TX_FILE);
    let (pending_tx_count, pending_error) = match std::fs::read(&pending_path) {
        Ok(bytes) => match parse_pending_file_bytes(&bytes) {
            Ok(v) => (Some(v.len()), None),
            Err(e) => (None, Some(e)),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Some(0), None),
        Err(e) => (None, Some(format!("{}: {e}", pending_path.display()))),
    };

    let wallet_address = load_wallet(data_dir).ok().map(|w| w.address().to_string());

    let peer_path = node::peer_book::PeerBook::path_in_data_dir(data_dir);
    let peer_seed_addrs = std::fs::read_to_string(&peer_path)
        .ok()
        .and_then(|s| {
            #[derive(serde::Deserialize)]
            struct File {
                #[serde(default)]
                peers: Vec<PeerAddr>,
            }
            #[derive(serde::Deserialize)]
            struct PeerAddr {
                addr: String,
            }
            toml::from_str::<File>(&s).ok()
        })
        .map(|f| f.peers.into_iter().map(|p| p.addr).collect())
        .unwrap_or_default();

    let mut blocks = vec![];
    if let Some(ref chain) = chain_result {
        let n = chain.blocks().len().min(15);
        let start = chain.blocks().len().saturating_sub(n);
        for b in chain.blocks().iter().skip(start) {
            blocks.push(BlockSummary {
                height: b.height,
                block_hash: b.block_hash.clone(),
                tx_count: b.transactions.len(),
                timestamp_unix: b.timestamp_unix,
            });
        }
    }

    let (height, tip_hash) = chain_result
        .as_ref()
        .map(|c| (c.height(), c.blocks().last().map(|b| b.block_hash.clone()).unwrap_or_default()))
        .unwrap_or((0, String::new()));

    Ok(OverviewOut {
        warning: WARN,
        data_dir: data_dir.display().to_string(),
        height,
        tip_hash,
        chain_error,
        pending_tx_count,
        pending_error,
        node_run_active,
        mempool_note: MEMPOOL_NOTE,
        wallet_address,
        peer_seed_addrs,
        blocks,
    })
}

async fn get_activity(State(st): State<AppState>) -> impl IntoResponse {
    let data_dir = st.data_dir.clone();
    let res = tokio::task::spawn_blocking(move || activity_inner(&data_dir)).await;
    match res {
        Ok(Ok(json)) => (StatusCode::OK, Json(json)).into_response(),
        Ok(Err(e)) => (
            StatusCode::OK,
            Json(ActivityOut {
                warning: WARN,
                blocks: vec![],
                recent_txs: vec![],
                error: Some(e),
            }),
        )
        .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "task join failed"})),
        )
            .into_response(),
    }
}

fn activity_inner(data_dir: &Path) -> Result<ActivityOut, String> {
    let genesis = load_genesis(data_dir)?;
    let chain = load_chain_readonly(data_dir, &genesis)?;
    let mut blocks = vec![];
    let mut recent_txs = vec![];
    let n = chain.blocks().len().min(20);
    let start = chain.blocks().len().saturating_sub(n);
    for b in chain.blocks().iter().skip(start) {
        blocks.push(BlockSummary {
            height: b.height,
            block_hash: b.block_hash.clone(),
            tx_count: b.transactions.len(),
            timestamp_unix: b.timestamp_unix,
        });
        for tx in &b.transactions {
            if recent_txs.len() >= 40 {
                break;
            }
            recent_txs.push(TxSummary {
                tx_hash: tx.tx_hash.clone(),
                sender: tx.sender.to_string(),
                receiver: tx.receiver.to_string(),
                amount: tx.amount,
                fee: tx.fee,
                nonce: tx.nonce,
            });
        }
    }
    Ok(ActivityOut {
        warning: WARN,
        blocks,
        recent_txs,
        error: None,
    })
}

async fn get_account(State(st): State<AppState>, Query(q): Query<AccountQuery>) -> impl IntoResponse {
    let data_dir = st.data_dir.clone();
    let addr = q.addr;
    let res = tokio::task::spawn_blocking(move || account_inner(&data_dir, &addr)).await;
    match res {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "task join failed"})),
        )
            .into_response(),
    }
}

fn account_inner(data_dir: &Path, addr: &str) -> AccountOut {
    let genesis = match load_genesis(data_dir) {
        Ok(g) => g,
        Err(e) => {
            return AccountOut {
                address: addr.to_string(),
                found: false,
                balance: None,
                nonce: None,
                error: Some(e),
            };
        }
    };
    let chain = match load_chain_readonly(data_dir, &genesis) {
        Ok(c) => c,
        Err(e) => {
            return AccountOut {
                address: addr.to_string(),
                found: false,
                balance: None,
                nonce: None,
                error: Some(e),
            };
        }
    };
    let a = Address::new(addr);
    match chain.state().get_account(&a) {
        Some(ac) => AccountOut {
            address: addr.to_string(),
            found: true,
            balance: Some(ac.balance),
            nonce: Some(ac.nonce),
            error: None,
        },
        None => AccountOut {
            address: addr.to_string(),
            found: false,
            balance: None,
            nonce: None,
            error: None,
        },
    }
}

async fn get_wallet(State(st): State<AppState>) -> impl IntoResponse {
    let data_dir = st.data_dir.clone();
    let res = tokio::task::spawn_blocking(move || wallet_state_inner(&data_dir)).await;
    match res {
        Ok(json) => (StatusCode::OK, Json(json)).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(WalletOut {
                address: None,
                balance: None,
                committed_next_nonce: None,
                error: Some("task join failed".into()),
            }),
        )
            .into_response(),
    }
}

fn wallet_state_inner(data_dir: &Path) -> WalletOut {
    let wallet = match load_wallet(data_dir) {
        Ok(w) => w,
        Err(e) => {
            return WalletOut {
                address: None,
                balance: None,
                committed_next_nonce: None,
                error: Some(e),
            };
        }
    };
    let addr = wallet.address().to_string();
    let genesis = match load_genesis(data_dir) {
        Ok(g) => g,
        Err(e) => {
            return WalletOut {
                address: Some(addr),
                balance: None,
                committed_next_nonce: None,
                error: Some(e),
            };
        }
    };
    let chain = match load_chain_readonly(data_dir, &genesis) {
        Ok(c) => c,
        Err(e) => {
            return WalletOut {
                address: Some(addr),
                balance: None,
                committed_next_nonce: None,
                error: Some(e),
            };
        }
    };
    match chain.state().get_account(&wallet.address()) {
        Some(ac) => WalletOut {
            address: Some(addr),
            balance: Some(ac.balance),
            committed_next_nonce: Some(ac.nonce),
            error: None,
        },
        None => WalletOut {
            address: Some(addr),
            balance: None,
            committed_next_nonce: None,
            error: Some(
                "wallet address has no account on this chain (check genesis allocations)".into(),
            ),
        },
    }
}

async fn get_setup(State(st): State<AppState>) -> Json<SetupOut> {
    let data_dir = st.data_dir.display().to_string();
    let dd = st.data_dir.clone();
    Json(SetupOut {
        warning: WARN,
        data_dir: data_dir.clone(),
        paths: PathsOut {
            genesis: dd.join(GENESIS_FILE).display().to_string(),
            chain_blocks: dd.join(CHAIN_FILE).display().to_string(),
            pending_tx: dd.join(PENDING_TX_FILE).display().to_string(),
            wallet_seed: dd.join(WALLET_FILE).display().to_string(),
            genesis_bind: dd.join(data_dir_bind::GENESIS_BIND_FILE)
                .display()
                .to_string(),
            run_lock: dd.join(NODE_RUN_LOCK_FILE).display().to_string(),
            peer_book: node::peer_book::PeerBook::path_in_data_dir(&dd)
                .display()
                .to_string(),
        },
        example_commands: vec![
            format!(
                "cd ../node && cargo run -- run --data-dir {} --listen 127.0.0.1:9333 --interval-secs 2",
                dd.display()
            ),
            format!(
                "cargo run --manifest-path dev-test-ui/Cargo.toml -- --data-dir {}",
                dd.display()
            ),
        ],
    })
}

async fn post_send(
    State(st): State<AppState>,
    Json(body): Json<SendBody>,
) -> impl IntoResponse {
    let data_dir = st.data_dir.clone();
    let res = tokio::task::spawn_blocking(move || send_inner(&data_dir, body)).await;
    match res {
        Ok(Ok(msg)) => (StatusCode::OK, Json(serde_json::json!({"ok": true, "message": msg}))).into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": e})),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": "task join failed"})),
        )
            .into_response(),
    }
}

fn send_inner(data_dir: &Path, body: SendBody) -> Result<String, String> {
    let wallet = load_wallet(data_dir)?;
    let genesis = load_genesis(data_dir)?;
    data_dir_bind::verify_binding_if_present(data_dir, &genesis)?;
    let chain = load_chain_readonly(data_dir, &genesis)?;
    let acct = chain
        .state()
        .get_account(&wallet.address())
        .ok_or_else(|| {
            "wallet address has no account on this chain genesis; fix genesis.toml".to_string()
        })?;
    let committed_nonce = acct.nonce;
    let nonce = body.nonce.unwrap_or(committed_nonce);
    let ts = unix_now_secs();
    let tx = wallet
        .sign_transfer(Address::new(&body.receiver), body.amount, body.fee, nonce, ts)
        .map_err(|e| e.to_string())?;
    let pending = data_dir.join(PENDING_TX_FILE);
    append_pending_transaction(&pending, &tx).map_err(|e| e.to_string())?;
    data_dir_bind::ensure_binding_if_missing(data_dir, &genesis).map_err(|e| e.to_string())?;

    let mut msg = format!(
        "queued tx {} -> {} amount {} fee {} nonce {} (pending_tx.tril — run `node run` to seal)",
        tx.tx_hash, body.receiver, body.amount, body.fee, nonce
    );
    if body.nonce.is_some() {
        msg.push_str(&format!(
            "\nNOTE: dev-test override nonce — committed next nonce was {committed_nonce}. \
             Stale (nonce < {committed_nonce}) or gap (nonce > {committed_nonce}) txs exercise mempool/seal rejection paths; see stderr on `node run`."
        ));
    }
    let total = body.amount.checked_add(body.fee);
    if total.map(|t| t > acct.balance).unwrap_or(true) {
        msg.push_str(&format!(
            "\nNOTE: amount+fee ({:?}) vs committed balance {} — expect seal or hygiene to reject insufficient balance.",
            total,
            acct.balance
        ));
    }
    Ok(msg)
}

fn usage(bin: &str) {
    eprintln!(
        "LOCAL TESTING ONLY — {bin}\n\
         Usage: {bin} [--data-dir DIR] [--listen ADDR:PORT]\n\
         Default: --data-dir ./data-a  --listen 127.0.0.1:9847\n\
         Open the URL in a browser; keep `node run` in another terminal for sealing."
    );
}

#[tokio::main]
async fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        usage(&std::env::args().next().unwrap_or_else(|| "trilogicon-dev-test-ui".into()));
        return;
    }

    let mut data_dir = PathBuf::from("data-a");
    let mut listen: String = "127.0.0.1:9847".into();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" => {
                data_dir = PathBuf::from(
                    args.get(i + 1)
                        .expect("--data-dir needs a path")
                        .clone(),
                );
                i += 2;
            }
            "--listen" => {
                listen = args.get(i + 1).expect("--listen needs ADDR:PORT").clone();
                i += 2;
            }
            _ => {
                eprintln!("unknown arg: {}", args[i]);
                usage(&std::env::args().next().unwrap_or_else(|| "trilogicon-dev-test-ui".into()));
                std::process::exit(1);
            }
        }
    }

    if !listen.starts_with("127.0.0.1:") && listen != "127.0.0.1" {
        eprintln!(
            "Refusing to bind {listen}: use 127.0.0.1 only (local testing tool)."
        );
        std::process::exit(1);
    }
    let listen = if listen == "127.0.0.1" {
        "127.0.0.1:9847".to_string()
    } else {
        listen
    };

    let state = AppState {
        data_dir: std::fs::canonicalize(&data_dir).unwrap_or(data_dir),
    };

    eprintln!(
        "[dev-test-ui] LOCAL TESTING ONLY | data-dir={} | http://{listen}/",
        state.data_dir.display()
    );
    eprintln!("[dev-test-ui] not a wallet; not for production; bind localhost only");

    let app = Router::new()
        .route("/", get(|| async { Html(include_str!("../static/index.html")) }))
        .route("/api/overview", get(get_overview))
        .route("/api/activity", get(get_activity))
        .route("/api/account", get(get_account))
        .route("/api/wallet", get(get_wallet))
        .route("/api/setup", get(get_setup))
        .route("/api/send", post(post_send))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .unwrap_or_else(|e| panic!("bind {listen}: {e}"));
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("serve");
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("\n[dev-test-ui] shutdown");
}
