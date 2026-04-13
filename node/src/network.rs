//! V1/V2 TCP peer protocol: length-prefixed frames, canonical encodings ([`crate::encoding`]).
//!
//! V2 adds a mandatory **session handshake** (genesis commitment + wire version), **bounded**
//! linear sync (including per-call [`SyncWorkBudget`] and fresh `now_unix` per appended block),
//! **outbound** read/write deadlines on peer sockets, **inbound** lifecycle limits (caps, idle
//! read timeout, protocol-error budget, **invalid decodable-block** budget for `OP_BLOCK`), and
//! gossip helpers. See `docs/design_notes/v2_wire_peer_sync.md` and
//! `docs/design_notes/v2_network_defense.md`.
//!
//! Opcodes (first payload byte after outer frame length):
//! - `1` — transaction body = [`encode_transaction`]
//! - `2` — block body = [`encode_block`]
//! - `3` — get blocks: next 8 bytes = `u64_be` `start_height` (reply is opcode `4`)
//! - `4` — block batch: `u32_be` count, then count × (`u32_be` len + block bytes)
//! - `5` — session HELLO (V2)
//! - `6` — session HELLO_ACK (V2)

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::block::Block;
use crate::blockchain::Blockchain;
use crate::encoding::{decode_block, decode_transaction, encode_block, encode_transaction};
use crate::genesis::Genesis;
use crate::mempool::Mempool;
use crate::operator_msg::{PFX_MEMPOOL, PFX_PEER, PFX_STORAGE, PFX_SYNC};
use crate::storage::BlockStore;
use crate::transaction::Transaction;

/// Outer framed message size cap (hostile allocation bound).
pub const MAX_WIRE_FRAME_BYTES: u32 = 64 * 1024 * 1024;

/// Maximum blocks in one `OP_BLOCKS` batch (defensive; not a consensus rule).
pub const MAX_BLOCKS_PER_BATCH: u32 = 64;

/// Wire protocol version for session handshake (bump when handshake semantics change).
pub const TRIL_WIRE_PROTOCOL_VERSION: u16 = 2;

/// Max UTF-8 bytes for genesis commitment hex in session frames.
pub const MAX_SESSION_COMMITMENT_BYTES: usize = 512;

/// Outbound TCP connect timeout (operational).
pub const PEER_CONNECT_TIMEOUT_SECS: u64 = 10;

/// Default outbound write stall limit (slow reader / full socket buffer). Local-only; not consensus.
pub const PEER_OUTBOUND_WRITE_TIMEOUT_SECS: u64 = 60;

/// Per [`sync_from_peer`] call: max TCP pull rounds (each round is one connect + GET_BLOCKS + reply).
pub const DEFAULT_SYNC_MAX_ROUNDS_PER_CALL: u32 = 8192;

/// Per [`sync_from_peer`] call: max blocks appended across all rounds in that call.
pub const DEFAULT_SYNC_MAX_BLOCKS_PER_CALL: u32 = 500_000;

/// Per [`sync_from_peer`] call: max total bytes read from wire (response frames) in that call.
pub const DEFAULT_SYNC_MAX_WIRE_BYTES_PER_CALL: u64 = 512 * 1024 * 1024;

/// Prefix still included in the final [`io::Error`] message when the invalid-block budget trips (operator grep / logs).
/// Session control flow uses [`PeerFrameError::InvalidBlockBudgetExhausted`], not substring matching on this text.
pub const INVALID_BLOCK_BUDGET_EXHAUSTED: &str = "TRIL_INVALID_BLOCK_BUDGET_EXHAUSTED";

/// Operator-visible tag when [`PeerFrameError::StaleBlockIngressQuotaExhausted`] disconnects the session.
pub const INGRESS_STALE_BLOCK_QUOTA_EXHAUSTED: &str = "TRIL_INGRESS_STALE_BLOCK_QUOTA_EXHAUSTED";

/// Operator-visible tag when [`PeerFrameError::InboundTxIngressQuotaExhausted`] disconnects the session.
pub const INGRESS_INBOUND_TX_QUOTA_EXHAUSTED: &str = "TRIL_INGRESS_INBOUND_TX_QUOTA_EXHAUSTED";

/// Why [`NodeInner::append_network_block_persist`] failed. Drives inbound invalid-block strike policy **without** parsing error strings.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum NetworkBlockPersistFailure {
    /// [`BlockStore`] poisoned before attempting the append.
    StorePoisoned,
    /// `try_append_network_block` rejected the peer-supplied block (validity / tip / time rules).
    PeerRejectedBlock { detail: String },
    /// Internal invariant after a successful RAM append.
    MissingTipAfterApply,
    /// `chain.blocks` append failed after RAM commit; in-memory tip was rolled back.
    PersistAfterApply { io_detail: String },
    /// Append failed and rollback failed (local state may be inconsistent).
    RollbackFailed {
        persist_detail: String,
        rollback_detail: String,
    },
}

impl NetworkBlockPersistFailure {
    /// When `true`, one inbound [`InboundPeerPolicy::max_invalid_network_blocks_per_session`] strike is recorded.
    pub fn counts_toward_invalid_block_budget(&self) -> bool {
        matches!(self, Self::PeerRejectedBlock { .. })
    }
}

impl std::fmt::Display for NetworkBlockPersistFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StorePoisoned => write!(
                f,
                "{PFX_STORAGE} fail-closed: store poisoned after a prior append/sync failure — refusing further appends in this process; restart after repairing chain.blocks"
            ),
            Self::PeerRejectedBlock { detail } => f.write_str(detail),
            Self::MissingTipAfterApply => f.write_str("missing tip after append"),
            Self::PersistAfterApply { io_detail } => write!(
                f,
                "{PFX_STORAGE} persist block failed: {io_detail} — chain.blocks may be partial; store is fail-closed for this process until restart"
            ),
            Self::RollbackFailed {
                persist_detail,
                rollback_detail,
            } => write!(
                f,
                "FATAL: persist failed: {persist_detail}; chain rollback failed: {rollback_detail} (disk and memory may disagree — stop the node and repair chain.blocks)"
            ),
        }
    }
}

/// Post-handshake frame handling outcome for peer session lifecycle (disconnect vs protocol-error budget).
#[derive(Debug)]
pub enum PeerFrameError {
    /// Invalid-block strike budget exhausted — disconnect immediately (do not count as generic protocol error).
    InvalidBlockBudgetExhausted { max_invalid_network_blocks_per_session: u32 },
    /// Decoded `OP_BLOCK` with `height <= local_tip` exceeded [`InboundPeerPolicy::max_stale_decoded_blocks_per_session`].
    StaleBlockIngressQuotaExhausted {
        max_stale_decoded_blocks_per_session: u32,
    },
    /// Successfully decoded inbound `OP_TX` count exceeded [`InboundPeerPolicy::max_inbound_tx_per_session`].
    InboundTxIngressQuotaExhausted {
        max_inbound_tx_per_session: u32,
    },
    /// Local persist / poison / rollback — fail-closed for this session.
    LocalFatal(String),
    /// Malformed opcode, decode failure, etc. — counts toward [`InboundPeerPolicy::max_protocol_errors_per_session`].
    Protocol(io::Error),
}

/// Outbound peer socket deadlines (gossip, sync client). Separate from inbound [`InboundPeerPolicy`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutboundPeerTimeouts {
    pub read: Option<Duration>,
    pub write: Option<Duration>,
}

impl Default for OutboundPeerTimeouts {
    fn default() -> Self {
        Self {
            read: Some(Duration::from_secs(90)),
            write: Some(Duration::from_secs(PEER_OUTBOUND_WRITE_TIMEOUT_SECS)),
        }
    }
}

/// Apply outbound read/write deadlines before handshake or payload writes on a connected peer socket.
pub fn apply_outbound_stream_timeouts(stream: &mut TcpStream, timeouts: &OutboundPeerTimeouts) {
    stream.set_read_timeout(timeouts.read).ok();
    stream.set_write_timeout(timeouts.write).ok();
}

/// Local work budget for one [`sync_from_peer`] invocation (not consensus; catch-up resumes later).
///
/// **`max_wire_bytes_per_call` is a soft cap:** bytes are counted after each full `OP_BLOCKS` read.
/// If a read pushes the counter at or over the cap, the **already-received** batch is still
/// validated and appended (subject to [`Self::max_blocks_per_call`]) before this call returns with
/// `stopped_due_to_budget == true`. The next `sync_from_peer` starts with a fresh
/// byte counter so catch-up cannot stall forever solely because one response was larger than the cap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncWorkBudget {
    pub max_rounds_per_call: u32,
    pub max_blocks_per_call: u32,
    pub max_wire_bytes_per_call: u64,
}

impl Default for SyncWorkBudget {
    fn default() -> Self {
        Self {
            max_rounds_per_call: DEFAULT_SYNC_MAX_ROUNDS_PER_CALL,
            max_blocks_per_call: DEFAULT_SYNC_MAX_BLOCKS_PER_CALL,
            max_wire_bytes_per_call: DEFAULT_SYNC_MAX_WIRE_BYTES_PER_CALL,
        }
    }
}

/// Reject pathological budgets that would forbid any progress (`0` caps). Use large values (e.g.
/// `u64::MAX` for bytes) for “effectively unlimited” operational limits.
pub fn validate_sync_work_budget(budget: &SyncWorkBudget) -> Result<(), String> {
    if budget.max_rounds_per_call == 0 {
        return Err(format!(
            "{PFX_SYNC} invalid budget: max_rounds_per_call must be >= 1"
        ));
    }
    if budget.max_blocks_per_call == 0 {
        return Err(format!(
            "{PFX_SYNC} invalid budget: max_blocks_per_call must be >= 1"
        ));
    }
    if budget.max_wire_bytes_per_call == 0 {
        return Err(format!(
            "{PFX_SYNC} invalid budget: max_wire_bytes_per_call must be >= 1 (use u64::MAX for a practical unlimited cap)"
        ));
    }
    Ok(())
}

/// Result of [`sync_from_peer`]: how much was appended and whether local caps stopped early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncFromPeerOutcome {
    pub blocks_appended: usize,
    pub stopped_due_to_budget: bool,
}

pub const OP_TX: u8 = 1;
pub const OP_BLOCK: u8 = 2;
pub const OP_GET_BLOCKS: u8 = 3;
pub const OP_BLOCKS: u8 = 4;
pub const OP_SESSION_HELLO: u8 = 5;
pub const OP_SESSION_HELLO_ACK: u8 = 6;

/// Local operational limits for **inbound** peer sessions (not consensus).
#[derive(Clone, Copy, Debug)]
pub struct InboundPeerPolicy {
    /// Concurrent TCP sessions allowed after accept (handshake runs inside the slot).
    pub max_concurrent_sessions: usize,
    /// Per-read stall limit while waiting for the next full frame (idle / stuck peer).
    pub idle_read_timeout: Duration,
    /// Applied to outbound writes on this socket; `None` disables the write deadline.
    pub write_timeout: Option<Duration>,
    /// After handshake: disconnect when this many post-handshake protocol violations occur
    /// (e.g. unknown opcode, bad `GET_BLOCKS` length). Oversized wire frames fail closed immediately.
    pub max_protocol_errors_per_session: u32,
    /// Hard cap on handled application frames per session (defense in depth).
    pub max_app_frames_per_session: u32,
    /// After a decodable `OP_BLOCK`, rejections that are not benign stale height and not local
    /// persistence failures increment this budget; exhaustion disconnects (local defense only).
    pub max_invalid_network_blocks_per_session: u32,
    /// Decoded `OP_BLOCK` with `height <= local_tip` (no append attempt): bounds decode/hash work from
    /// stale replay spam. Distinct from [`Self::max_invalid_network_blocks_per_session`].
    pub max_stale_decoded_blocks_per_session: u32,
    /// Successfully decoded inbound [`OP_TX`] per session (each may run `basic_validate` / mempool admit).
    pub max_inbound_tx_per_session: u32,
}

impl Default for InboundPeerPolicy {
    fn default() -> Self {
        Self {
            max_concurrent_sessions: 128,
            idle_read_timeout: Duration::from_secs(120),
            write_timeout: Some(Duration::from_secs(60)),
            max_protocol_errors_per_session: 32,
            max_app_frames_per_session: 100_000,
            max_invalid_network_blocks_per_session: 24,
            max_stale_decoded_blocks_per_session: 8192,
            max_inbound_tx_per_session: 100_000,
        }
    }
}

/// Tracks concurrent inbound sessions for [`serve_tcp_listener`].
pub struct InboundSlotPool {
    max: usize,
    active: AtomicUsize,
}

impl InboundSlotPool {
    pub fn new(max: usize) -> Arc<Self> {
        Arc::new(Self {
            max,
            active: AtomicUsize::new(0),
        })
    }

    pub fn max_slots(&self) -> usize {
        self.max
    }

    /// Returns `None` when [`InboundPeerPolicy::max_concurrent_sessions`] is reached.
    pub fn try_acquire(self: &Arc<Self>) -> Option<InboundPermit> {
        let mut n = self.active.load(Ordering::SeqCst);
        loop {
            if n >= self.max {
                return None;
            }
            match self.active.compare_exchange_weak(
                n,
                n + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Some(InboundPermit { pool: Arc::clone(self) }),
                Err(x) => n = x,
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn active_sessions_for_tests(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }
}

pub struct InboundPermit {
    pool: Arc<InboundSlotPool>,
}

impl Drop for InboundPermit {
    fn drop(&mut self) {
        // Never use `fetch_sub` here: on `active == 0` it wraps to `usize::MAX` and breaks the cap.
        loop {
            let c = self.pool.active.load(Ordering::SeqCst);
            if c == 0 {
                eprintln!("{PFX_PEER} inbound slot release while active count was 0 (should not happen)");
                break;
            }
            if self
                .pool
                .active
                .compare_exchange(c, c - 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }
    }
}

fn io_err_idle_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

/// Shared node state for the TCP listener and the block-production loop.
pub struct NodeInner {
    /// Used to roll back the in-memory tip if `chain.blocks` append fails after a successful RAM append.
    pub genesis: Genesis,
    pub chain: Blockchain,
    pub pool: Mempool,
    pub store: BlockStore,
}

impl NodeInner {
    pub fn append_network_block_persist(
        &mut self,
        block: Block,
        now_unix: u64,
    ) -> Result<(), NetworkBlockPersistFailure> {
        if self.store.is_poisoned() {
            return Err(NetworkBlockPersistFailure::StorePoisoned);
        }
        self.chain
            .try_append_network_block(block, now_unix)
            .map_err(|e| NetworkBlockPersistFailure::PeerRejectedBlock {
                detail: e.to_string(),
            })?;
        let tip = self
            .chain
            .blocks()
            .last()
            .ok_or(NetworkBlockPersistFailure::MissingTipAfterApply)?
            .clone();
        match self.store.append_block(&tip) {
            Ok(()) => Ok(()),
            Err(e) => {
                let persist_detail = e.to_string();
                if let Err(r) = self.chain.rollback_last_block(&self.genesis) {
                    return Err(NetworkBlockPersistFailure::RollbackFailed {
                        persist_detail,
                        rollback_detail: r.to_string(),
                    });
                }
                Err(NetworkBlockPersistFailure::PersistAfterApply {
                    io_detail: persist_detail,
                })
            }
        }
    }

    /// Local mempool hygiene after the committed ledger advanced (catch-up or inbound block).
    pub fn mempool_hygiene_after_ledger_advance(&mut self) -> (usize, usize, usize) {
        self.pool.hygiene_vs_committed_ledger(self.chain.state())
    }
}

pub fn unix_now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn read_framed(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut hdr = [0u8; 4];
    reader.read_exact(&mut hdr)?;
    let len = u32::from_be_bytes(hdr) as usize;
    if len as u32 > MAX_WIRE_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "wire frame exceeds max size",
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    Ok(body)
}

/// Validates outbound framed body length: same cap as [`read_framed`] so we never emit frames peers must reject.
pub fn validate_outbound_framed_payload_len(payload_len: usize) -> io::Result<u32> {
    if payload_len > MAX_WIRE_FRAME_BYTES as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "wire payload exceeds MAX_WIRE_FRAME_BYTES",
        ));
    }
    u32::try_from(payload_len).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "wire payload length does not fit u32",
        )
    })
}

pub fn write_framed(writer: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    let len = validate_outbound_framed_payload_len(payload.len())?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

/// Build session HELLO / HELLO_ACK body: opcode + wire version + commitment + advisory height.
pub fn encode_session_payload(
    opcode: u8,
    genesis: &Genesis,
    advisory_height: u64,
) -> Result<Vec<u8>, String> {
    let hex = genesis.state_commitment_hex().map_err(|e| e.to_string())?;
    if hex.len() > MAX_SESSION_COMMITMENT_BYTES {
        return Err(format!(
            "genesis commitment hex too long for wire session (max {MAX_SESSION_COMMITMENT_BYTES})"
        ));
    }
    let hlen = u16::try_from(hex.len()).map_err(|_| "commitment length overflow".to_string())?;
    let mut v = vec![opcode];
    v.extend_from_slice(&TRIL_WIRE_PROTOCOL_VERSION.to_be_bytes());
    v.extend_from_slice(&hlen.to_be_bytes());
    v.extend_from_slice(hex.as_bytes());
    v.extend_from_slice(&advisory_height.to_be_bytes());
    Ok(v)
}

/// Decode session frame. Returns `(opcode, wire_version, commitment_hex, advisory_height)`.
pub fn decode_session_payload(data: &[u8]) -> Result<(u8, u16, String, u64), String> {
    if data.len() < 1 + 2 + 2 + 8 {
        return Err(format!("{PFX_PEER} malformed session frame: too short"));
    }
    let op = data[0];
    let ver = u16::from_be_bytes([data[1], data[2]]);
    let hlen = u16::from_be_bytes([data[3], data[4]]) as usize;
    if hlen > MAX_SESSION_COMMITMENT_BYTES {
        return Err(format!(
            "{PFX_PEER} malformed session frame: commitment length exceeds cap"
        ));
    }
    if data.len() < 5 + hlen + 8 {
        return Err(format!("{PFX_PEER} malformed session frame: truncated"));
    }
    let hex_str = std::str::from_utf8(&data[5..5 + hlen])
        .map_err(|_| format!("{PFX_PEER} malformed session frame: commitment not valid UTF-8"))?;
    let advisory = u64::from_be_bytes(
        data[5 + hlen..5 + hlen + 8]
            .try_into()
            .map_err(|_| format!("{PFX_PEER} malformed session frame: advisory height bytes"))?,
    );
    let expected_len = 5 + hlen + 8;
    if data.len() != expected_len {
        return Err(format!(
            "{PFX_PEER} malformed session frame: trailing bytes after payload"
        ));
    }
    Ok((op, ver, hex_str.to_string(), advisory))
}

fn local_commitment_hex(genesis: &Genesis) -> Result<String, String> {
    genesis.state_commitment_hex().map_err(|e| e.to_string())
}

/// Initiator: send HELLO, read ACK, verify genesis + wire version.
///
/// Callers should set outbound read/write timeouts on `stream` before this if wedging must be bounded.
pub fn handshake_initiator(
    stream: &mut TcpStream,
    local_genesis: &Genesis,
    local_advisory_height: u64,
) -> io::Result<()> {
    let hello = encode_session_payload(OP_SESSION_HELLO, local_genesis, local_advisory_height)
        .map_err(|s| io::Error::new(io::ErrorKind::InvalidData, s))?;
    write_framed(stream, &hello)?;
    let resp = read_framed(stream)?;
    let (op, ver, peer_hex, peer_adv) = decode_session_payload(&resp)
        .map_err(|s| io::Error::new(io::ErrorKind::InvalidData, s))?;
    if op != OP_SESSION_HELLO_ACK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{PFX_PEER} disconnecting: expected session HELLO_ACK, got opcode {op}"),
        ));
    }
    if ver != TRIL_WIRE_PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{PFX_PEER} disconnecting: incompatible wire version (peer {ver}, want {TRIL_WIRE_PROTOCOL_VERSION})"
            ),
        ));
    }
    let local_hex = local_commitment_hex(local_genesis)
        .map_err(|s| io::Error::new(io::ErrorKind::InvalidData, s))?;
    if peer_hex != local_hex {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{PFX_PEER} disconnecting: genesis state commitment mismatch (align genesis.toml across nodes; not local chain.blocks corruption)"
            ),
        ));
    }
    eprintln!(
        "{PFX_PEER} session ok (outbound; wire v{ver}; peer advisory height {peer_adv} — advisory only, not used for sync bounds)"
    );
    Ok(())
}

/// Responder: read HELLO, verify, send ACK.
pub fn handshake_responder(
    stream: &mut TcpStream,
    local_genesis: &Genesis,
    local_advisory_height: u64,
) -> io::Result<()> {
    let frame = read_framed(stream)?;
    let (op, ver, peer_hex, peer_adv) = decode_session_payload(&frame)
        .map_err(|s| io::Error::new(io::ErrorKind::InvalidData, s))?;
    if op != OP_SESSION_HELLO {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{PFX_PEER} disconnecting: expected session HELLO (op {OP_SESSION_HELLO}) as first frame, got opcode {op}"
            ),
        ));
    }
    if ver != TRIL_WIRE_PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{PFX_PEER} disconnecting: incompatible wire version (peer {ver}, want {TRIL_WIRE_PROTOCOL_VERSION})"
            ),
        ));
    }
    let local_hex = local_commitment_hex(local_genesis)
        .map_err(|s| io::Error::new(io::ErrorKind::InvalidData, s))?;
    if peer_hex != local_hex {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{PFX_PEER} disconnecting: genesis state commitment mismatch (align genesis.toml across nodes; not local chain.blocks corruption)"
            ),
        ));
    }
    eprintln!(
        "{PFX_PEER} session ok (inbound; wire v{ver}; peer advisory height {peer_adv} — advisory only)"
    );
    let ack = encode_session_payload(OP_SESSION_HELLO_ACK, local_genesis, local_advisory_height)
        .map_err(|s| io::Error::new(io::ErrorKind::InvalidData, s))?;
    write_framed(stream, &ack)?;
    Ok(())
}

pub fn tcp_connect_peer(peer: &str) -> io::Result<TcpStream> {
    let mut addrs = peer
        .to_socket_addrs()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("{peer}: {e}")))?;
    let addr = addrs.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no socket addresses for {peer}"),
        )
    })?;
    TcpStream::connect_timeout(&addr, Duration::from_secs(PEER_CONNECT_TIMEOUT_SECS))
}

/// Payload only (no outer frame): `OP_GET_BLOCKS` + `start_height`.
pub fn wire_encode_get_blocks(start_height: u64) -> Vec<u8> {
    let mut v = vec![OP_GET_BLOCKS];
    v.extend_from_slice(&start_height.to_be_bytes());
    v
}

/// Full response body: `OP_BLOCKS` + batch (at most `MAX_BLOCKS_PER_BATCH` blocks).
pub fn wire_encode_blocks_response(blocks: &[Block]) -> Result<Vec<u8>, String> {
    let mut out = vec![OP_BLOCKS];
    let n = u32::try_from(blocks.len().min(MAX_BLOCKS_PER_BATCH as usize))
        .map_err(|_| "block count overflow".to_string())?;
    let take = n as usize;
    out.extend_from_slice(&n.to_be_bytes());
    for b in blocks.iter().take(take) {
        let enc = encode_block(b);
        let len =
            u32::try_from(enc.len()).map_err(|_| "encoded block length overflow".to_string())?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&enc);
        if out.len() > MAX_WIRE_FRAME_BYTES as usize {
            return Err("encoded OP_BLOCKS response exceeds MAX_WIRE_FRAME_BYTES".into());
        }
    }
    Ok(out)
}

fn parse_op_blocks_body(data: &[u8], max_blocks: u32) -> Result<Vec<Block>, String> {
    if data.len() < 4 {
        return Err("short OP_BLOCKS body".into());
    }
    let n = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if n as u32 > max_blocks {
        return Err(format!(
            "OP_BLOCKS batch count {n} exceeds max {max_blocks}"
        ));
    }
    let mut pos = 4usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if pos + 4 > data.len() {
            return Err(format!("truncated block length ({i})"));
        }
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if pos + len > data.len() {
            return Err(format!("truncated block body ({i})"));
        }
        let block = decode_block(&data[pos..pos + len]).map_err(|e| e.to_string())?;
        out.push(block);
        pos += len;
    }
    if pos != data.len() {
        return Err("trailing bytes after OP_BLOCKS batch".into());
    }
    Ok(out)
}

/// Decode a full response frame body (must start with `OP_BLOCKS`).
pub fn wire_decode_blocks_response(payload: &[u8]) -> Result<Vec<Block>, String> {
    match payload.first() {
        Some(&OP_BLOCKS) => parse_op_blocks_body(&payload[1..], MAX_BLOCKS_PER_BATCH),
        Some(o) => Err(format!("expected OP_BLOCKS, got opcode {o}")),
        None => Err("empty response".into()),
    }
}

/// Reject batches that are not a strict contiguous linear extension from `chain` tip.
/// Also runs [`Block::basic_validate`] on each block (no state mutation).
pub fn validate_linear_sync_batch(chain: &Blockchain, blocks: &[Block]) -> Result<(), String> {
    if blocks.is_empty() {
        return Ok(());
    }
    let tip = chain
        .blocks()
        .last()
        .ok_or_else(|| "chain has no tip".to_string())?;
    let mut expect_height = tip.height.saturating_add(1);
    let mut prev_hash = tip.block_hash.clone();
    for (i, b) in blocks.iter().enumerate() {
        b.basic_validate()
            .map_err(|e| format!("sync batch block {i} basic_validate: {e}"))?;
        if b.height != expect_height {
            return Err(format!(
                "sync batch not linear at index {i}: expected height {expect_height}, got {} (gap, stale, or out-of-order)",
                b.height
            ));
        }
        if b.previous_hash != prev_hash {
            return Err(format!(
                "sync batch broken link at index {i}: previous_hash does not extend tip"
            ));
        }
        prev_hash = b.block_hash.clone();
        expect_height = expect_height.saturating_add(1);
    }
    Ok(())
}

/// Post-handshake application payload after **decode only** — no [`NodeInner`] / chain access.
/// Parsing runs outside the global node lock in [`peer_connection_loop`] to shorten lock hold time.
#[derive(Debug)]
pub enum PredecodedInbound {
    Tx(Transaction),
    Block(Block),
    GetBlocks { start_height: u64 },
}

/// Opcode check + transaction/block decode + `GET_BLOCKS` length parse. **Does not** touch node state
/// (safe to call without holding [`NodeInner`] mutex). Malformed payloads return [`PeerFrameError::Protocol`].
pub fn predecode_inbound_app_payload(payload: &[u8]) -> Result<PredecodedInbound, PeerFrameError> {
    let op = *payload.first().ok_or_else(|| {
        PeerFrameError::Protocol(io::Error::new(io::ErrorKind::InvalidData, "empty payload"))
    })?;

    match op {
        OP_TX => {
            let tx = decode_transaction(&payload[1..]).map_err(|e| {
                PeerFrameError::Protocol(io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
            })?;
            Ok(PredecodedInbound::Tx(tx))
        }
        OP_BLOCK => {
            let block = decode_block(&payload[1..]).map_err(|e| {
                PeerFrameError::Protocol(io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
            })?;
            Ok(PredecodedInbound::Block(block))
        }
        OP_GET_BLOCKS => {
            if payload.len() != 1 + 8 {
                return Err(PeerFrameError::Protocol(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{PFX_PEER} malformed GET_BLOCKS payload length"),
                )));
            }
            let mut h = [0u8; 8];
            h.copy_from_slice(&payload[1..9]);
            Ok(PredecodedInbound::GetBlocks {
                start_height: u64::from_be_bytes(h),
            })
        }
        OP_SESSION_HELLO | OP_SESSION_HELLO_ACK => Err(PeerFrameError::Protocol(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{PFX_PEER} session handshake must be first frame only"),
        ))),
        _ => Err(PeerFrameError::Protocol(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{PFX_PEER} unknown wire opcode"),
        ))),
    }
}

/// Apply a [`PredecodedInbound`] under the node lock (poison check, tip, mempool, append, serve blocks).
fn apply_predecoded(
    inner: &mut NodeInner,
    decoded: PredecodedInbound,
    now_unix: u64,
    invalid_block_strikes: &mut u32,
    stale_decoded_blocks: &mut u32,
    policy: &InboundPeerPolicy,
) -> Result<Option<Vec<u8>>, PeerFrameError> {
    if inner.store.is_poisoned() {
        return Err(PeerFrameError::LocalFatal(format!(
            "{PFX_STORAGE} fail-closed: store poisoned after write failure — closing peer session; repair chain.blocks and restart"
        )));
    }

    match decoded {
        PredecodedInbound::Tx(tx) => {
            if let Err(e) = inner.pool.try_submit(tx) {
                eprintln!("{PFX_PEER} mempool rejected inbound tx ({e}) — peer message dropped, session continues");
            }
            Ok(None)
        }
        PredecodedInbound::Block(block) => {
            let local_tip = inner.chain.height();
            if block.height <= local_tip {
                *stale_decoded_blocks = stale_decoded_blocks.saturating_add(1);
                if *stale_decoded_blocks > policy.max_stale_decoded_blocks_per_session {
                    return Err(PeerFrameError::StaleBlockIngressQuotaExhausted {
                        max_stale_decoded_blocks_per_session: policy
                            .max_stale_decoded_blocks_per_session,
                    });
                }
                eprintln!(
                    "{PFX_PEER} ignored stale block (height {} ≤ local tip {}) — no invalid-block strike (ingress stale quota {}/{})",
                    block.height,
                    local_tip,
                    *stale_decoded_blocks,
                    policy.max_stale_decoded_blocks_per_session
                );
                return Ok(None);
            }
            match inner.append_network_block_persist(block, now_unix) {
                Ok(()) => {
                    let (f, s, d) = inner.mempool_hygiene_after_ledger_advance();
                    if f > 0 || s > 0 || d > 0 {
                        eprintln!(
                            "{PFX_MEMPOOL} after inbound block (height {}): FIFO-cleaned {f}, stale-nonce dropped {s}, sender+nonce dup dropped {d} (local policy)",
                            inner.chain.height()
                        );
                    }
                    Ok(None)
                }
                Err(failure) => {
                    if failure.counts_toward_invalid_block_budget() {
                        *invalid_block_strikes = invalid_block_strikes.saturating_add(1);
                        eprintln!(
                            "{PFX_PEER} rejected network block ({failure}) — invalid-block strike {}/{} (local defense)",
                            *invalid_block_strikes,
                            policy.max_invalid_network_blocks_per_session
                        );
                        if *invalid_block_strikes >= policy.max_invalid_network_blocks_per_session {
                            return Err(PeerFrameError::InvalidBlockBudgetExhausted {
                                max_invalid_network_blocks_per_session: policy
                                    .max_invalid_network_blocks_per_session,
                            });
                        }
                        Ok(None)
                    } else {
                        eprintln!(
                            "{PFX_STORAGE} fail-closed: rejected block — local persist/store error ({failure}) — closing session"
                        );
                        Err(PeerFrameError::LocalFatal(failure.to_string()))
                    }
                }
            }
        }
        PredecodedInbound::GetBlocks { start_height } => {
            let slice = inner.chain.blocks_from_height_limited(
                start_height,
                MAX_BLOCKS_PER_BATCH as usize,
            );
            let encoded = wire_encode_blocks_response(&slice).map_err(|e| {
                PeerFrameError::Protocol(io::Error::new(io::ErrorKind::InvalidData, e))
            })?;
            Ok(Some(encoded))
        }
    }
}

fn peer_session_setup(stream: &mut TcpStream, state: &Arc<Mutex<NodeInner>>) -> io::Result<()> {
    let (genesis, height, poisoned) = {
        let g = state.lock().expect("node lock poisoned");
        (
            g.genesis.clone(),
            g.chain.height(),
            g.store.is_poisoned(),
        )
    };
    if poisoned {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "{PFX_STORAGE} fail-closed: store poisoned — refusing new inbound session until restart"
            ),
        ));
    }
    handshake_responder(stream, &genesis, height)
}

/// Handle framed messages on one inbound connection until EOF, idle timeout, or policy limit.
///
/// **Slot lifetime:** the caller must hold an [`InboundPermit`] for this connection for the whole
/// duration of this function (including V2 handshake). A peer that connects but never completes
/// handshake still occupies capacity until idle timeout or disconnect — same as post-handshake idle.
pub fn peer_connection_loop(
    stream: TcpStream,
    state: Arc<Mutex<NodeInner>>,
    policy: InboundPeerPolicy,
) -> io::Result<()> {
    let mut stream = stream;
    stream.set_read_timeout(Some(policy.idle_read_timeout)).ok();
    if let Some(w) = policy.write_timeout {
        stream.set_write_timeout(Some(w)).ok();
    }

    peer_session_setup(&mut stream, &state)?;

    let mut protocol_errors: u32 = 0;
    let mut app_frames: u32 = 0;
    let mut invalid_block_strikes: u32 = 0;
    let mut stale_decoded_blocks: u32 = 0;
    let mut inbound_tx_decoded: u32 = 0;

    loop {
        let frame = match read_framed(&mut stream) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) if io_err_idle_timeout(&e) => {
                eprintln!(
                    "{PFX_PEER} session idle read timeout ({}s) — closing connection (local policy)",
                    policy.idle_read_timeout.as_secs()
                );
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                return Err(io::Error::new(
                    e.kind(),
                    format!("{PFX_PEER} fail-closed: wire frame read rejected — {e}"),
                ));
            }
            Err(e) => return Err(e),
        };

        let Some(next_frame) = app_frames.checked_add(1) else {
            eprintln!("{PFX_PEER} session frame counter overflow — closing (local policy)");
            return Ok(());
        };
        app_frames = next_frame;
        if app_frames > policy.max_app_frames_per_session {
            eprintln!(
                "{PFX_PEER} session exceeded max_app_frames_per_session ({}) — closing (local policy)",
                policy.max_app_frames_per_session
            );
            return Ok(());
        }

        let now = unix_now_secs();
        let outcome = match predecode_inbound_app_payload(&frame) {
            Err(e) => Err(e),
            Ok(decoded) => {
                let tx_within_quota = match &decoded {
                    PredecodedInbound::Tx(_) => {
                        inbound_tx_decoded = inbound_tx_decoded.saturating_add(1);
                        inbound_tx_decoded <= policy.max_inbound_tx_per_session
                    }
                    _ => true,
                };
                if !tx_within_quota {
                    Err(PeerFrameError::InboundTxIngressQuotaExhausted {
                        max_inbound_tx_per_session: policy.max_inbound_tx_per_session,
                    })
                } else {
                    let mut g = state.lock().expect("node lock poisoned");
                    apply_predecoded(
                        &mut g,
                        decoded,
                        now,
                        &mut invalid_block_strikes,
                        &mut stale_decoded_blocks,
                        &policy,
                    )
                }
            }
        };

        match outcome {
            Ok(Some(resp)) => write_framed(&mut stream, &resp)?,
            Ok(None) => {}
            Err(PeerFrameError::InvalidBlockBudgetExhausted {
                max_invalid_network_blocks_per_session,
            }) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{INVALID_BLOCK_BUDGET_EXHAUSTED}: {PFX_PEER} disconnect — peer exceeded max_invalid_network_blocks_per_session ({max_invalid_network_blocks_per_session})",
                    ),
                ));
            }
            Err(PeerFrameError::StaleBlockIngressQuotaExhausted {
                max_stale_decoded_blocks_per_session,
            }) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{INGRESS_STALE_BLOCK_QUOTA_EXHAUSTED}: {PFX_PEER} disconnect — exceeded max_stale_decoded_blocks_per_session ({max_stale_decoded_blocks_per_session})",
                    ),
                ));
            }
            Err(PeerFrameError::InboundTxIngressQuotaExhausted {
                max_inbound_tx_per_session,
            }) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{INGRESS_INBOUND_TX_QUOTA_EXHAUSTED}: {PFX_PEER} disconnect — exceeded max_inbound_tx_per_session ({max_inbound_tx_per_session})",
                    ),
                ));
            }
            Err(PeerFrameError::LocalFatal(msg)) => {
                // Local chain/store failure — fail closed, do not burn wire-error budget.
                return Err(io::Error::new(io::ErrorKind::Other, msg));
            }
            Err(PeerFrameError::Protocol(e)) => {
                protocol_errors = protocol_errors.saturating_add(1);
                eprintln!(
                    "{PFX_PEER} post-handshake protocol error ({}/{} before disconnect): {e}",
                    protocol_errors, policy.max_protocol_errors_per_session
                );
                if protocol_errors >= policy.max_protocol_errors_per_session {
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

pub fn serve_tcp_listener(
    listener: TcpListener,
    state: Arc<Mutex<NodeInner>>,
    policy: InboundPeerPolicy,
    slots: Arc<InboundSlotPool>,
) {
    loop {
        match listener.accept() {
            Ok((stream, addr)) => {
                let permit = match slots.try_acquire() {
                    Some(p) => p,
                    None => {
                        eprintln!(
                            "{PFX_PEER} inbound peer cap {} reached — refusing {}",
                            slots.max_slots(),
                            addr
                        );
                        drop(stream);
                        continue;
                    }
                };
                let st = state.clone();
                let pol = policy;
                thread::spawn(move || {
                    let _permit = permit;
                    if let Err(e) = peer_connection_loop(stream, st, pol) {
                        eprintln!("{PFX_PEER} session {addr} ended: {e}");
                    }
                });
            }
            Err(e) => eprintln!("{PFX_PEER} accept failed: {e}"),
        }
    }
}

/// Bind and spawn the accept loop. Returns the resolved address (e.g. with port when using `:0`).
pub fn spawn_incoming_loop(
    bind: &str,
    state: Arc<Mutex<NodeInner>>,
    policy: InboundPeerPolicy,
) -> io::Result<(thread::JoinHandle<()>, String)> {
    let listener = TcpListener::bind(bind)?;
    let addr = listener.local_addr()?.to_string();
    let slots = InboundSlotPool::new(policy.max_concurrent_sessions);
    let handle = thread::spawn(move || serve_tcp_listener(listener, state, policy, slots));
    Ok((handle, addr))
}

/// Connect, perform V2 session handshake, send `GET_BLOCKS`, read `OP_BLOCKS` reply.
/// Returns decoded blocks and the **response frame body length** (bytes) for sync work accounting.
pub fn pull_blocks_from_peer(
    peer: &str,
    start_height: u64,
    local_genesis: &Genesis,
    local_advisory_height: u64,
    timeouts: &OutboundPeerTimeouts,
) -> Result<(Vec<Block>, u64), String> {
    let mut stream =
        tcp_connect_peer(peer).map_err(|e| format!("{PFX_SYNC} connect {peer} failed: {e}"))?;
    apply_outbound_stream_timeouts(&mut stream, timeouts);
    handshake_initiator(&mut stream, local_genesis, local_advisory_height)
        .map_err(|e| format!("{PFX_SYNC} session handshake with {peer} failed: {e}"))?;
    let req = wire_encode_get_blocks(start_height);
    write_framed(&mut stream, &req).map_err(|e| e.to_string())?;
    let resp = read_framed(&mut stream).map_err(|e| e.to_string())?;
    let nbytes = resp.len() as u64;
    let blocks = wire_decode_blocks_response(&resp)?;
    Ok((blocks, nbytes))
}

/// Fetch blocks with height >= `chain.height() + 1` and append each in order after batch validation.
/// Issues multiple `GET_BLOCKS` rounds until the peer returns an **empty** batch (caught up), or a
/// [`SyncWorkBudget`] cap is hit (resume on a later call). Uses a fresh `now_unix` for **each**
/// appended block so long catch-up does not reuse a stale clock for drift checks.
pub fn sync_from_peer(
    inner: &mut NodeInner,
    peer: &str,
    budget: &SyncWorkBudget,
) -> Result<SyncFromPeerOutcome, String> {
    sync_from_peer_with_clock(inner, peer, budget, || unix_now_secs())
}

pub fn sync_from_peer_with_clock<F: Fn() -> u64>(
    inner: &mut NodeInner,
    peer: &str,
    budget: &SyncWorkBudget,
    now_unix: F,
) -> Result<SyncFromPeerOutcome, String> {
    if inner.store.is_poisoned() {
        return Err(format!(
            "{PFX_STORAGE} fail-closed: store poisoned — refusing peer sync; repair chain.blocks and restart"
        ));
    }
    validate_sync_work_budget(budget)?;

    let timeouts = OutboundPeerTimeouts::default();
    let mut total = 0usize;
    let mut rounds: u32 = 0;
    let mut wire_bytes: u64 = 0;
    let mut stopped = false;

    loop {
        if total >= budget.max_blocks_per_call as usize {
            stopped = true;
            break;
        }
        if rounds >= budget.max_rounds_per_call {
            stopped = true;
            break;
        }
        // Byte budget limits *additional* pulls in this call only after prior responses are processed.
        if wire_bytes >= budget.max_wire_bytes_per_call {
            stopped = true;
            break;
        }

        let height_now = inner.chain.height();
        let start = height_now.saturating_add(1);
        let (blocks, nbytes) = pull_blocks_from_peer(
            peer,
            start,
            &inner.genesis,
            height_now,
            &timeouts,
        )?;
        rounds = rounds.saturating_add(1);
        wire_bytes = wire_bytes.saturating_add(nbytes);

        if blocks.is_empty() {
            break;
        }

        validate_linear_sync_batch(&inner.chain, &blocks)?;

        let room = budget.max_blocks_per_call.saturating_sub(total as u32) as usize;
        if room == 0 {
            stopped = true;
            break;
        }

        let batch_len = blocks.len();
        let apply_n = batch_len.min(room);
        for b in blocks.into_iter().take(apply_n) {
            let now = now_unix();
            inner
                .append_network_block_persist(b, now)
                .map_err(|f| f.to_string())?;
            total += 1;
            if total >= budget.max_blocks_per_call as usize {
                stopped = true;
                break;
            }
        }

        if stopped {
            break;
        }
        if apply_n < batch_len {
            stopped = true;
            break;
        }

        // Soft cap: never discard a fetched batch unread, but do not pull again this call once over.
        if wire_bytes > budget.max_wire_bytes_per_call {
            stopped = true;
            break;
        }
    }

    if total > 0 {
        let (f, s, d) = inner.mempool_hygiene_after_ledger_advance();
        if f > 0 || s > 0 || d > 0 {
            eprintln!(
                "{PFX_MEMPOOL} after peer sync ({total} block(s)): FIFO-cleaned {f}, stale-nonce dropped {s}, sender+nonce dup dropped {d} (local policy)"
            );
        }
    }

    Ok(SyncFromPeerOutcome {
        blocks_appended: total,
        stopped_due_to_budget: stopped,
    })
}

/// Push one block to a peer (gossip). Uses [`OutboundPeerTimeouts::default`].
pub fn push_block_to_peer(
    peer: &str,
    local_genesis: &Genesis,
    local_advisory_height: u64,
    block: &Block,
) -> Result<(), String> {
    push_block_to_peer_with_timeouts(
        peer,
        local_genesis,
        local_advisory_height,
        block,
        &OutboundPeerTimeouts::default(),
    )
}

pub fn push_block_to_peer_with_timeouts(
    peer: &str,
    local_genesis: &Genesis,
    local_advisory_height: u64,
    block: &Block,
    timeouts: &OutboundPeerTimeouts,
) -> Result<(), String> {
    let mut stream =
        tcp_connect_peer(peer).map_err(|e| format!("{PFX_PEER} gossip connect {peer} failed: {e}"))?;
    apply_outbound_stream_timeouts(&mut stream, timeouts);
    handshake_initiator(&mut stream, local_genesis, local_advisory_height).map_err(|e| {
        format!("{PFX_PEER} gossip session handshake with {peer} failed: {e}")
    })?;
    let mut msg = vec![OP_BLOCK];
    msg.extend_from_slice(&encode_block(block));
    write_framed(&mut stream, &msg).map_err(|e| e.to_string())
}

pub fn push_tx_to_peer(
    peer: &str,
    local_genesis: &Genesis,
    local_advisory_height: u64,
    tx: &Transaction,
) -> Result<(), String> {
    push_tx_to_peer_with_timeouts(
        peer,
        local_genesis,
        local_advisory_height,
        tx,
        &OutboundPeerTimeouts::default(),
    )
}

pub fn push_tx_to_peer_with_timeouts(
    peer: &str,
    local_genesis: &Genesis,
    local_advisory_height: u64,
    tx: &Transaction,
    timeouts: &OutboundPeerTimeouts,
) -> Result<(), String> {
    let mut stream =
        tcp_connect_peer(peer).map_err(|e| format!("{PFX_PEER} push_tx connect {peer} failed: {e}"))?;
    apply_outbound_stream_timeouts(&mut stream, timeouts);
    handshake_initiator(&mut stream, local_genesis, local_advisory_height).map_err(|e| {
        format!("{PFX_PEER} push_tx session handshake with {peer} failed: {e}")
    })?;
    let mut msg = vec![OP_TX];
    msg.extend_from_slice(&encode_transaction(tx));
    write_framed(&mut stream, &msg).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
    use crate::genesis::{Genesis, GenesisAllocation};
    use crate::types::Address;
    use ed25519_dalek::{Signer, SigningKey};

    fn one_valid_block() -> Block {
        let signing_key = SigningKey::from_bytes(&[77u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let mut tx = Transaction {
            sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
            receiver: Address::new("recv_net"),
            amount: 2,
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_700_200_000,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let p = tx.unsigned_payload_bytes();
        tx.signature = signing_key.sign(&p).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&p);

        let mut b = Block {
            height: 1,
            previous_hash: "GENESIS_HASH".into(),
            timestamp_unix: 1_700_200_001,
            transactions: vec![tx],
            block_hash: String::new(),
        };
        b.block_hash = b.compute_block_hash();
        b
    }

    #[test]
    fn wire_blocks_roundtrip() {
        let b = one_valid_block();
        let w = wire_encode_blocks_response(std::slice::from_ref(&b)).unwrap();
        let out = wire_decode_blocks_response(&w).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].block_hash, b.block_hash);
    }

    #[test]
    fn session_payload_roundtrip() {
        let g = Genesis {
            allocations: vec![GenesisAllocation {
                address: Address::new("alice_sess").0,
                balance: 10,
            }],
        };
        let p = encode_session_payload(OP_SESSION_HELLO, &g, 42).unwrap();
        let (op, ver, hex, adv) = decode_session_payload(&p).unwrap();
        assert_eq!(op, OP_SESSION_HELLO);
        assert_eq!(ver, TRIL_WIRE_PROTOCOL_VERSION);
        assert_eq!(adv, 42);
        assert_eq!(hex, g.state_commitment_hex().unwrap());
    }

    #[test]
    fn inbound_slot_pool_acquire_release() {
        let p = InboundSlotPool::new(2);
        let a = p.try_acquire().unwrap();
        let _b = p.try_acquire().unwrap();
        assert!(p.try_acquire().is_none());
        assert_eq!(p.active_sessions_for_tests(), 2);
        drop(a);
        assert_eq!(p.active_sessions_for_tests(), 1);
        assert!(p.try_acquire().is_some());
    }

    #[test]
    fn validate_outbound_framed_payload_len_rejects_over_cap_without_alloc() {
        assert!(validate_outbound_framed_payload_len(MAX_WIRE_FRAME_BYTES as usize).is_ok());
        assert_eq!(
            validate_outbound_framed_payload_len((MAX_WIRE_FRAME_BYTES as usize).saturating_add(1))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            validate_outbound_framed_payload_len(usize::MAX)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn session_hello_rejects_trailing_bytes() {
        let g = Genesis {
            allocations: vec![GenesisAllocation {
                address: Address::new("trail_hello").0,
                balance: 1,
            }],
        };
        let mut p = encode_session_payload(OP_SESSION_HELLO, &g, 0).unwrap();
        p.push(0);
        let err = decode_session_payload(&p).unwrap_err();
        assert!(err.contains("trailing"), "{err}");
    }

    #[test]
    fn session_ack_rejects_trailing_bytes() {
        let g = Genesis {
            allocations: vec![GenesisAllocation {
                address: Address::new("trail_ack").0,
                balance: 1,
            }],
        };
        let mut p = encode_session_payload(OP_SESSION_HELLO_ACK, &g, 0).unwrap();
        p.push(0xff);
        let err = decode_session_payload(&p).unwrap_err();
        assert!(err.contains("trailing"), "{err}");
    }

    #[test]
    fn wire_encode_blocks_response_rejects_when_encoded_exceeds_frame_cap() {
        let mut b = one_valid_block();
        // One field expansion is enough to push `encode_block` past the wire cap (OP_BLOCKS header + block wrapper add overhead).
        b.transactions[0].public_key = vec![0u8; MAX_WIRE_FRAME_BYTES as usize];
        let err = wire_encode_blocks_response(std::slice::from_ref(&b)).unwrap_err();
        assert!(
            err.contains("MAX_WIRE_FRAME_BYTES"),
            "unexpected err: {err}"
        );
    }

    #[test]
    fn validate_linear_batch_rejects_gap() {
        let g = Genesis::empty();
        let chain = Blockchain::from_genesis(&g).unwrap();
        let b1 = one_valid_block();
        let mut b2 = one_valid_block();
        b2.height = 3;
        b2.previous_hash = b1.block_hash.clone();
        b2.block_hash = b2.compute_block_hash();
        let err = validate_linear_sync_batch(&chain, &[b1, b2]).unwrap_err();
        assert!(err.contains("not linear"), "{err}");
    }

    #[test]
    fn op_blocks_rejects_over_max_count() {
        let mut body = vec![0u8; 4];
        let over = MAX_BLOCKS_PER_BATCH + 1;
        body[0..4].copy_from_slice(&over.to_be_bytes());
        let err = parse_op_blocks_body(&body, MAX_BLOCKS_PER_BATCH).unwrap_err();
        assert!(err.contains("exceeds max"), "{err}");
    }

    #[test]
    fn validate_sync_work_budget_rejects_zero_caps() {
        let e1 = validate_sync_work_budget(&SyncWorkBudget {
            max_rounds_per_call: 0,
            max_blocks_per_call: 1,
            max_wire_bytes_per_call: 1,
        })
        .unwrap_err();
        assert!(e1.contains("[sync]"), "{e1}");
        let e2 = validate_sync_work_budget(&SyncWorkBudget {
            max_rounds_per_call: 1,
            max_blocks_per_call: 0,
            max_wire_bytes_per_call: 1,
        })
        .unwrap_err();
        assert!(e2.contains("[sync]"), "{e2}");
        let e3 = validate_sync_work_budget(&SyncWorkBudget {
            max_rounds_per_call: 1,
            max_blocks_per_call: 1,
            max_wire_bytes_per_call: 0,
        })
        .unwrap_err();
        assert!(e3.contains("[sync]"), "{e3}");
        assert!(validate_sync_work_budget(&SyncWorkBudget::default()).is_ok());
    }

    #[test]
    fn decode_session_payload_short_input_tags_peer() {
        let err = decode_session_payload(&[0u8; 4]).unwrap_err();
        assert!(err.contains("[peer]"), "{err}");
    }

    #[test]
    fn predecode_malformed_fails_without_any_node_mutex() {
        assert!(matches!(
            predecode_inbound_app_payload(&[]),
            Err(PeerFrameError::Protocol(_))
        ));
        assert!(matches!(
            predecode_inbound_app_payload(&[0xff]),
            Err(PeerFrameError::Protocol(_))
        ));
    }

    #[test]
    fn predecode_get_blocks_fixed_layout_without_node_state() {
        let mut ok = vec![OP_GET_BLOCKS];
        ok.extend_from_slice(&42u64.to_be_bytes());
        match predecode_inbound_app_payload(&ok).unwrap() {
            PredecodedInbound::GetBlocks { start_height } => assert_eq!(start_height, 42),
            _ => panic!("expected GetBlocks"),
        }
        let short = vec![OP_GET_BLOCKS, 1, 2, 3];
        assert!(matches!(
            predecode_inbound_app_payload(&short),
            Err(PeerFrameError::Protocol(_))
        ));
    }

    #[test]
    fn invalid_block_strike_uses_typed_failure_not_message_heuristics() {
        assert!(
            NetworkBlockPersistFailure::PeerRejectedBlock {
                detail: "arbitrary wording that must not be substring-scanned".into(),
            }
            .counts_toward_invalid_block_budget()
        );
        assert!(
            !NetworkBlockPersistFailure::StorePoisoned.counts_toward_invalid_block_budget()
        );
        assert!(
            !NetworkBlockPersistFailure::MissingTipAfterApply.counts_toward_invalid_block_budget()
        );
        assert!(
            !NetworkBlockPersistFailure::PersistAfterApply {
                io_detail: "persist block failed: anything".into(),
            }
            .counts_toward_invalid_block_budget()
        );
        assert!(
            !NetworkBlockPersistFailure::RollbackFailed {
                persist_detail: "e".into(),
                rollback_detail: "r".into(),
            }
            .counts_toward_invalid_block_budget()
        );
    }

    #[test]
    fn peer_frame_error_invalid_block_budget_is_distinct_variant() {
        let e = PeerFrameError::InvalidBlockBudgetExhausted {
            max_invalid_network_blocks_per_session: 7,
        };
        assert!(matches!(
            e,
            PeerFrameError::InvalidBlockBudgetExhausted {
                max_invalid_network_blocks_per_session: 7
            }
        ));
    }

    #[test]
    fn sync_from_peer_refuses_poisoned_store() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "trilog_net_poison_{}_{}",
            std::process::id(),
            nanos
        ));
        let _ = std::fs::remove_file(&path);
        let g = Genesis::empty();
        let chain = Blockchain::from_genesis(&g).unwrap();
        let store = BlockStore::open_append(&path).unwrap();
        let mut inner = NodeInner {
            genesis: g,
            chain,
            pool: Mempool::new(10),
            store,
        };
        inner.store.mark_poisoned_for_tests();
        let err = sync_from_peer(&mut inner, "127.0.0.1:9", &SyncWorkBudget::default()).unwrap_err();
        assert!(
            err.contains("poisoned") && err.contains("[storage]") && err.contains("fail-closed"),
            "unexpected err: {err}"
        );
        let _ = std::fs::remove_file(&path);
    }
}
