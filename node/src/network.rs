//! TCP peer protocol: length-prefixed frames, canonical encodings ([`crate::encoding`]).
//!
//! Opcodes (first payload byte after frame length):
//! - `1` — transaction body
//! - `2` — block body
//! - `3` — get blocks: next 8 bytes = `u64_be` `start_height` (reply is opcode `4`)
//! - `4` — block batch
//! - `5` — HELLO (v2 handshake: network id + genesis commitment + tip metadata)
//! - `6` — HELLO_OK (response)
//! - `7` — REJECT (optional short reason after handshake failure)
//! - `8` — REQUEST_PEERS (empty body; reply `OP_PEERS`)
//! - `9` — PEERS (`u32_be` count, then per peer `u16_be` addr len + UTF-8 host:port)
//! - `10` — BLOCK_INV (`u64_be` height + `u16_be` hash len + UTF-8 block hash)
//! - `11` — BLOCK_WANT (same layout; request full `OP_BLOCK` reply)
//! - `12` — INV_ACK (no body; decline / already have / not serving)
//!
//! Long catch-up: [`sync_from_peer`] may issue several `GET_BLOCKS` rounds (cap [`MAX_SYNC_PULL_ROUNDS`])
//! because each `OP_BLOCKS` batch is bounded ([`MAX_BLOCKS_PER_WIRE_BATCH`]). Total blocks applied
//! per call is capped at [`MAX_BLOCKS_APPLIED_PER_SYNC`] (tighter than [`MAX_BLOCKS_APPLIED_PER_SYNC_WIRE_MAX`]).
//! Opcode tables for tooling: [`docs/wire_protocol.md`](../../docs/wire_protocol.md) in the repo root.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::block::Block;
use crate::blockchain::Blockchain;
use crate::diag;
use crate::encoding::{decode_block, decode_transaction, encode_block, encode_transaction};
use crate::genesis::Genesis;
use crate::mempool::Mempool;
use crate::peer_book::PeerBook;
use crate::seen::SeenCache;
use crate::storage::BlockStore;
use crate::transaction::Transaction;

/// Prefix on [`sync_from_peer`] / [`append_network_block_persist`] errors when chain state may be inconsistent.
pub const FATAL_SYNC_PREFIX: &str = "FATAL:";

pub const MAX_WIRE_FRAME_BYTES: u32 = 64 * 1024 * 1024;

/// Upper bound on how many blocks a peer may send in one `OP_BLOCKS` batch (anti-DoS).
pub const MAX_BLOCKS_PER_WIRE_BATCH: u32 = 4096;

/// Max framed messages processed on one inbound TCP session (anti-DoS).
pub const MAX_FRAMES_PER_INBOUND_SESSION: u32 = 8192;

/// Max `GET_BLOCKS` → `OP_BLOCKS` pull rounds per [`sync_from_peer`] (anti-runaway).
pub const MAX_SYNC_PULL_ROUNDS: u32 = 256;

/// Upper bound on blocks applied in one [`sync_from_peer`] implied by pull rounds × batch size only.
pub const MAX_BLOCKS_APPLIED_PER_SYNC_WIRE_MAX: u32 =
    MAX_SYNC_PULL_ROUNDS * MAX_BLOCKS_PER_WIRE_BATCH;

/// Max blocks appended in one [`sync_from_peer`] call (across all pull rounds).
///
/// Kept below [`MAX_BLOCKS_APPLIED_PER_SYNC_WIRE_MAX`] to bound memory and CPU latency per sync while
/// still allowing multiple full batches per run.
pub const MAX_BLOCKS_APPLIED_PER_SYNC: u32 = 262_144;

const _: () = assert!(MAX_BLOCKS_APPLIED_PER_SYNC <= MAX_BLOCKS_APPLIED_PER_SYNC_WIRE_MAX);

/// Cap on peer addresses in one [`OP_PEERS`] frame (anti-DoS).
pub const MAX_PEERS_PER_WIRE_FRAME: u32 = 64;
/// Max UTF-8 bytes for one peer address in [`OP_PEERS`].
pub const MAX_PEER_ADDR_WIRE_BYTES: usize = 512;

/// Wire protocol version carried in HELLO (increment when handshake semantics change).
pub const TRIL_WIRE_VERSION: u16 = 2;

pub const MAX_TIP_HASH_WIRE_BYTES: usize = 256;

const OP_TX: u8 = 1;
const OP_BLOCK: u8 = 2;
const OP_GET_BLOCKS: u8 = 3;
const OP_BLOCKS: u8 = 4;
pub const OP_HELLO: u8 = 5;
pub const OP_HELLO_OK: u8 = 6;
pub const OP_REJECT: u8 = 7;
pub const OP_REQUEST_PEERS: u8 = 8;
pub const OP_PEERS: u8 = 9;
pub const OP_BLOCK_INV: u8 = 10;
pub const OP_BLOCK_WANT: u8 = 11;
pub const OP_INV_ACK: u8 = 12;

/// Tunable wire / handshake policy (v2). Does not change transaction or block validity rules.
#[derive(Clone, Debug)]
pub struct WireRuntimeConfig {
    pub wire_version: u16,
    pub network_id: u32,
    pub genesis_commitment: [u8; 32],
    /// Send [`OP_HELLO`] before other messages on outbound connections.
    pub handshake_outbound: bool,
    /// Inbound: first frame must be [`OP_HELLO`].
    pub require_handshake_inbound: bool,
    /// Inbound: allow pre-v2 peers that send legacy first frames.
    pub allow_legacy_inbound: bool,
    /// After block sync, request [`OP_PEERS`] and merge into [`PeerBook`] (requires compatible peer).
    pub exchange_peers: bool,
    /// Push new blocks as [`OP_BLOCK_INV`] first; peer may reply [`OP_BLOCK_WANT`] for full [`OP_BLOCK`].
    pub announce_blocks: bool,
}

impl Default for WireRuntimeConfig {
    fn default() -> Self {
        Self {
            wire_version: TRIL_WIRE_VERSION,
            network_id: 1,
            genesis_commitment: [0u8; 32],
            handshake_outbound: false,
            require_handshake_inbound: false,
            allow_legacy_inbound: true,
            exchange_peers: false,
            announce_blocks: false,
        }
    }
}

impl WireRuntimeConfig {
    pub fn from_genesis(
        genesis: &Genesis,
        network_id: u32,
        handshake_outbound: bool,
    ) -> Result<Self, String> {
        let genesis_commitment = genesis
            .state_commitment_raw32()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            wire_version: TRIL_WIRE_VERSION,
            network_id,
            genesis_commitment,
            handshake_outbound,
            require_handshake_inbound: false,
            allow_legacy_inbound: true,
            exchange_peers: false,
            announce_blocks: false,
        })
    }

    pub fn with_inbound_policy(mut self, require_handshake: bool, allow_legacy: bool) -> Self {
        self.require_handshake_inbound = require_handshake;
        self.allow_legacy_inbound = allow_legacy;
        self
    }

    pub fn with_gossip_extensions(mut self, exchange_peers: bool, announce_blocks: bool) -> Self {
        self.exchange_peers = exchange_peers;
        self.announce_blocks = announce_blocks;
        self
    }
}

/// Shared node state for the TCP listener and the block-production loop.
pub struct NodeInner {
    /// Same document used to build the initial chain; required to roll back memory after a failed disk append.
    pub genesis: Genesis,
    pub wire: WireRuntimeConfig,
    pub chain: Blockchain,
    pub pool: Mempool,
    pub store: BlockStore,
    pub seen_tx: SeenCache,
    pub seen_block: SeenCache,
    pub peer_book: PeerBook,
}

impl NodeInner {
    /// Test / library helper: default wire (no outbound handshake), standard seen cache sizes.
    pub fn for_tests(
        genesis: Genesis,
        chain: Blockchain,
        pool: Mempool,
        store: BlockStore,
    ) -> Result<Self, String> {
        let wire = WireRuntimeConfig::from_genesis(&genesis, 1, false)?;
        Ok(Self {
            genesis,
            wire,
            chain,
            pool,
            store,
            seen_tx: SeenCache::new(50_000),
            seen_block: SeenCache::new(50_000),
            peer_book: PeerBook::default(),
        })
    }

    /// Apply a gossiped block and persist it. If the store append fails after a successful in-memory append,
    /// rolls the chain back to `height_before` so RAM matches the last durable tip.
    pub fn append_network_block_persist(
        &mut self,
        block: Block,
        now_unix: u64,
    ) -> Result<(), String> {
        let height_before = self.chain.height();
        self.chain
            .try_append_network_block(block, now_unix)
            .map_err(|e| e.to_string())?;
        let tip = self
            .chain
            .blocks()
            .last()
            .ok_or_else(|| "missing tip after append".to_string())?
            .clone();
        if let Err(e) = self.store.append_block(&tip) {
            if let Err(rerr) = self.chain.rollback_to_height(height_before, &self.genesis) {
                return Err(format!(
                    "{FATAL_SYNC_PREFIX} persist failed ({e}) and rollback failed ({rerr}); chain state may be inconsistent"
                ));
            }
            return Err(format!("persist inbound block height {}: {e}", tip.height));
        }
        Ok(())
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

pub fn write_framed(writer: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "wire payload length does not fit u32",
        )
    })?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

/// Framed HELLO payload (opcode + body) for tests and tooling.
pub fn wire_payload_hello(
    wire: &WireRuntimeConfig,
    tip_height: u64,
    tip_hash: &str,
) -> Result<Vec<u8>, String> {
    let body = encode_hello_body(
        wire.wire_version,
        wire.network_id,
        &wire.genesis_commitment,
        tip_height,
        tip_hash,
    )?;
    let mut v = vec![OP_HELLO];
    v.extend_from_slice(&body);
    Ok(v)
}

/// Framed HELLO_OK payload for tests and mini-servers.
pub fn wire_payload_hello_ok(
    wire: &WireRuntimeConfig,
    tip_height: u64,
    tip_hash: &str,
) -> Result<Vec<u8>, String> {
    let body = encode_hello_body(
        wire.wire_version,
        wire.network_id,
        &wire.genesis_commitment,
        tip_height,
        tip_hash,
    )?;
    let mut v = vec![OP_HELLO_OK];
    v.extend_from_slice(&body);
    Ok(v)
}

fn encode_hello_body(
    wire_version: u16,
    network_id: u32,
    genesis_commitment: &[u8; 32],
    tip_height: u64,
    tip_hash: &str,
) -> Result<Vec<u8>, String> {
    let th = tip_hash.as_bytes();
    if th.len() > MAX_TIP_HASH_WIRE_BYTES {
        return Err("tip hash too long for wire".into());
    }
    let mut v = Vec::with_capacity(2 + 4 + 32 + 8 + 2 + th.len());
    v.extend_from_slice(&wire_version.to_be_bytes());
    v.extend_from_slice(&network_id.to_be_bytes());
    v.extend_from_slice(genesis_commitment);
    v.extend_from_slice(&tip_height.to_be_bytes());
    v.extend_from_slice(&(th.len() as u16).to_be_bytes());
    v.extend_from_slice(th);
    Ok(v)
}

fn decode_hello_body(data: &[u8]) -> Result<(u16, u32, [u8; 32], u64, String), String> {
    if data.len() < 2 + 4 + 32 + 8 + 2 {
        return Err("short HELLO body".into());
    }
    let mut i = 0usize;
    let wire_version = u16::from_be_bytes([data[i], data[i + 1]]);
    i += 2;
    let network_id = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
    i += 4;
    let mut g = [0u8; 32];
    g.copy_from_slice(&data[i..i + 32]);
    i += 32;
    let tip_height = u64::from_be_bytes(data[i..i + 8].try_into().map_err(|_| "hello tip_height")?);
    i += 8;
    let thlen = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
    i += 2;
    if thlen > MAX_TIP_HASH_WIRE_BYTES {
        return Err("HELLO tip hash length excessive".into());
    }
    if i + thlen > data.len() {
        return Err("truncated HELLO tip hash".into());
    }
    let tip_hash = String::from_utf8_lossy(&data[i..i + thlen]).into_owned();
    i += thlen;
    if i != data.len() {
        return Err("trailing HELLO bytes".into());
    }
    Ok((wire_version, network_id, g, tip_height, tip_hash))
}

/// Client: send HELLO and read HELLO_OK; validates peer agrees on version, network, genesis.
pub fn outbound_handshake(
    stream: &mut TcpStream,
    wire: &WireRuntimeConfig,
    tip_height: u64,
    tip_hash: &str,
) -> Result<(), String> {
    let body = encode_hello_body(
        wire.wire_version,
        wire.network_id,
        &wire.genesis_commitment,
        tip_height,
        tip_hash,
    )?;
    let mut payload = vec![OP_HELLO];
    payload.extend_from_slice(&body);
    write_framed(stream, &payload).map_err(|e| e.to_string())?;
    let resp = read_framed(stream).map_err(|e| e.to_string())?;
    match resp.first() {
        Some(&OP_HELLO_OK) => {}
        Some(&OP_REJECT) => {
            let msg = if resp.len() > 3 {
                String::from_utf8_lossy(&resp[3..]).into_owned()
            } else {
                "rejected".into()
            };
            return Err(format!("handshake rejected: {msg}"));
        }
        Some(o) => return Err(format!("handshake: expected HELLO_OK, got opcode {o}")),
        None => return Err("handshake: empty response".into()),
    }
    let (ver, net, genesis_bytes, _, _) = decode_hello_body(&resp[1..])?;
    if ver != wire.wire_version {
        return Err(format!(
            "handshake: peer wire version {ver} != ours {}",
            wire.wire_version
        ));
    }
    if net != wire.network_id {
        return Err("handshake: network_id mismatch after HELLO_OK".into());
    }
    if genesis_bytes != wire.genesis_commitment {
        return Err("handshake: genesis commitment mismatch after HELLO_OK".into());
    }
    Ok(())
}

fn build_hello_ok(inner: &NodeInner) -> Result<Vec<u8>, String> {
    let tip = inner
        .chain
        .blocks()
        .last()
        .ok_or_else(|| "chain tip missing".to_string())?;
    let body = encode_hello_body(
        inner.wire.wire_version,
        inner.wire.network_id,
        &inner.wire.genesis_commitment,
        tip.height,
        &tip.block_hash,
    )?;
    let mut out = vec![OP_HELLO_OK];
    out.extend_from_slice(&body);
    Ok(out)
}

fn validate_remote_hello_against_local(inner: &NodeInner, data: &[u8]) -> Result<(), String> {
    let (ver, net, genesis_bytes, _, _) = decode_hello_body(data)?;
    if ver != inner.wire.wire_version {
        return Err(format!(
            "wire version {ver} (ours {})",
            inner.wire.wire_version
        ));
    }
    if net != inner.wire.network_id {
        return Err(format!("network_id {net} (ours {})", inner.wire.network_id));
    }
    if genesis_bytes != inner.wire.genesis_commitment {
        return Err("genesis commitment mismatch".into());
    }
    Ok(())
}

/// Payload only (no outer frame): `OP_GET_BLOCKS` + `start_height`.
pub fn wire_encode_get_blocks(start_height: u64) -> Vec<u8> {
    let mut v = vec![OP_GET_BLOCKS];
    v.extend_from_slice(&start_height.to_be_bytes());
    v
}

/// Full response body: `OP_BLOCKS` + batch (for tests and servers).
pub fn wire_encode_blocks_response(blocks: &[Block]) -> Result<Vec<u8>, String> {
    let mut out = vec![OP_BLOCKS];
    let n = u32::try_from(blocks.len()).map_err(|_| "block count overflow".to_string())?;
    out.extend_from_slice(&n.to_be_bytes());
    for b in blocks {
        let enc = encode_block(b);
        let len =
            u32::try_from(enc.len()).map_err(|_| "encoded block length overflow".to_string())?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&enc);
    }
    Ok(out)
}

fn parse_op_blocks_body(data: &[u8]) -> Result<Vec<Block>, String> {
    if data.len() < 4 {
        return Err("short OP_BLOCKS body".into());
    }
    let n_raw = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    if n_raw > MAX_BLOCKS_PER_WIRE_BATCH {
        return Err(format!(
            "OP_BLOCKS batch count {n_raw} exceeds max {MAX_BLOCKS_PER_WIRE_BATCH}"
        ));
    }
    let n = n_raw as usize;
    let mut pos = 4usize;
    let mut out = Vec::new();
    for i in 0..n {
        if pos + 4 > data.len() {
            return Err(format!("truncated block length ({i})"));
        }
        let len_raw = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;
        if len_raw > MAX_WIRE_FRAME_BYTES {
            return Err(format!(
                "OP_BLOCKS entry {i}: encoded block length exceeds max frame size"
            ));
        }
        let len = len_raw as usize;
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
        Some(&OP_BLOCKS) => parse_op_blocks_body(&payload[1..]),
        Some(o) => Err(format!("expected OP_BLOCKS, got opcode {o}")),
        None => Err("empty response".into()),
    }
}

fn wire_encode_peers(book: &PeerBook) -> Vec<u8> {
    let mut addrs: Vec<String> = book
        .gossip_addresses(MAX_PEERS_PER_WIRE_FRAME as usize)
        .into_iter()
        .filter(|a| a.len() <= MAX_PEER_ADDR_WIRE_BYTES)
        .collect();
    addrs.truncate(MAX_PEERS_PER_WIRE_FRAME as usize);
    let mut out = vec![OP_PEERS];
    let n = u32::try_from(addrs.len()).unwrap_or(0);
    out.extend_from_slice(&n.to_be_bytes());
    for a in addrs {
        let b = a.as_bytes();
        if let Ok(len) = u16::try_from(b.len()) {
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(b);
        }
    }
    out
}

fn wire_decode_peers_body(body: &[u8]) -> Result<Vec<String>, String> {
    if body.len() < 4 {
        return Err("short OP_PEERS body".into());
    }
    let n_raw = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
    if n_raw > MAX_PEERS_PER_WIRE_FRAME {
        return Err(format!(
            "OP_PEERS count {n_raw} exceeds max {MAX_PEERS_PER_WIRE_FRAME}"
        ));
    }
    let n = n_raw as usize;
    let mut pos = 4usize;
    let mut out = Vec::new();
    for i in 0..n {
        if pos + 2 > body.len() {
            return Err(format!("truncated peer addr len ({i})"));
        }
        let al = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
        pos += 2;
        if al > MAX_PEER_ADDR_WIRE_BYTES {
            return Err(format!("peer addr {i} too long"));
        }
        if pos + al > body.len() {
            return Err(format!("truncated peer addr ({i})"));
        }
        let s = std::str::from_utf8(&body[pos..pos + al])
            .map_err(|_| format!("peer addr {i} not utf8"))?;
        if !s.is_empty() {
            out.push(s.to_string());
        }
        pos += al;
    }
    if pos != body.len() {
        return Err("trailing bytes after OP_PEERS".into());
    }
    Ok(out)
}

/// Decode a frame whose first byte is [`OP_PEERS`].
pub fn wire_decode_peers_response(payload: &[u8]) -> Result<Vec<String>, String> {
    match payload.first() {
        Some(&OP_PEERS) => wire_decode_peers_body(&payload[1..]),
        Some(o) => Err(format!("expected OP_PEERS, got opcode {o}")),
        None => Err("empty PEERS response".into()),
    }
}

fn encode_height_hash_wire(op: u8, height: u64, hash: &str) -> Result<Vec<u8>, String> {
    let hb = hash.as_bytes();
    if hb.len() > MAX_TIP_HASH_WIRE_BYTES {
        return Err("block hash too long for wire".into());
    }
    let mut v = vec![op];
    v.extend_from_slice(&height.to_be_bytes());
    v.extend_from_slice(&(hb.len() as u16).to_be_bytes());
    v.extend_from_slice(hb);
    Ok(v)
}

fn decode_height_hash_body(data: &[u8]) -> Result<(u64, String), String> {
    if data.len() < 8 + 2 {
        return Err("short height/hash wire body".into());
    }
    let height = u64::from_be_bytes(data[0..8].try_into().map_err(|_| "height")?);
    let hl = u16::from_be_bytes([data[8], data[9]]) as usize;
    if hl > MAX_TIP_HASH_WIRE_BYTES {
        return Err("hash length excessive".into());
    }
    if 10 + hl > data.len() {
        return Err("truncated block hash".into());
    }
    let hash = std::str::from_utf8(&data[10..10 + hl])
        .map_err(|_| "block hash not utf8".to_string())?
        .to_string();
    if 10 + hl != data.len() {
        return Err("trailing bytes after height/hash".into());
    }
    Ok((height, hash))
}

/// Request [`OP_PEERS`] over a new connection (after optional outbound handshake). Does nothing if
/// [`WireRuntimeConfig::exchange_peers`] is false.
pub fn exchange_peers_with_peer(inner: &mut NodeInner, peer: &str) -> Result<usize, String> {
    if !inner.wire.exchange_peers {
        return Ok(0);
    }
    let (tip_height, tip_hash) = {
        let tip = inner
            .chain
            .blocks()
            .last()
            .ok_or_else(|| "missing chain tip".to_string())?;
        (tip.height, tip.block_hash.clone())
    };
    let mut stream = TcpStream::connect(peer).map_err(|e| format!("connect {peer}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(60))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(60))).ok();
    if inner.wire.handshake_outbound {
        outbound_handshake(&mut stream, &inner.wire, tip_height, &tip_hash)?;
    }
    write_framed(&mut stream, &[OP_REQUEST_PEERS]).map_err(|e| e.to_string())?;
    let resp = read_framed(&mut stream).map_err(|e| e.to_string())?;
    let addrs = wire_decode_peers_response(&resp)?;
    Ok(inner.peer_book.merge_seeds_report_new(addrs))
}

fn process_payload(
    payload: &[u8],
    inner: &mut NodeInner,
    now_unix: u64,
    remote_peer: Option<&str>,
) -> io::Result<Option<Vec<u8>>> {
    let op = *payload
        .first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty payload"))?;

    match op {
        OP_TX => {
            let tx = decode_transaction(&payload[1..])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            let id = tx.tx_hash.clone();
            if !inner.seen_tx.insert(id) {
                return Ok(None);
            }
            if let Err(e) = inner.pool.try_submit(tx) {
                diag::line("mempool", format!("rejected tx ({e})"));
            }
            Ok(None)
        }
        OP_BLOCK => {
            let block = decode_block(&payload[1..])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            if !inner.seen_block.insert(block.block_hash.clone()) {
                return Ok(None);
            }
            match inner.append_network_block_persist(block, now_unix) {
                Ok(()) => {}
                Err(e) if e.starts_with(FATAL_SYNC_PREFIX) => {
                    diag::line("fatal", e.as_str());
                    return Err(io::Error::other(e));
                }
                Err(e) => diag::line("block", format!("rejected ({e})")),
            }
            Ok(None)
        }
        OP_GET_BLOCKS => {
            if payload.len() != 1 + 8 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "GET_BLOCKS length",
                ));
            }
            let mut h = [0u8; 8];
            h.copy_from_slice(&payload[1..9]);
            let start_height = u64::from_be_bytes(h);
            let blocks = inner.chain.blocks_from_height(start_height);
            let encoded = wire_encode_blocks_response(&blocks)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            Ok(Some(encoded))
        }
        OP_REQUEST_PEERS => {
            if payload.len() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "REQUEST_PEERS body length",
                ));
            }
            Ok(Some(wire_encode_peers(&inner.peer_book)))
        }
        OP_PEERS => {
            let addrs = wire_decode_peers_body(&payload[1..])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let _new = inner.peer_book.merge_seeds_report_new(addrs);
            Ok(None)
        }
        OP_BLOCK_INV => {
            let (h, hash) = decode_height_hash_body(&payload[1..])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let my_h = inner.chain.height();
            if h <= my_h {
                return Ok(Some(vec![OP_INV_ACK]));
            }
            if let Some(tip) = inner.chain.blocks().last() {
                if tip.height == h && tip.block_hash == hash {
                    return Ok(Some(vec![OP_INV_ACK]));
                }
            }
            if h == my_h.saturating_add(1) {
                let body = encode_height_hash_wire(OP_BLOCK_WANT, h, &hash)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                return Ok(Some(body));
            }
            if h > my_h.saturating_add(1) {
                let from = remote_peer.unwrap_or("?");
                diag::line(
                    "block_inv",
                    format!(
                        "from {from}: announce height {h} hash {hash} (our tip {my_h}); catch up via periodic sync"
                    ),
                );
            }
            Ok(Some(vec![OP_INV_ACK]))
        }
        OP_BLOCK_WANT => {
            let (h, want_hash) = decode_height_hash_body(&payload[1..])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let idx = usize::try_from(h)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "height index"))?;
            let Some(b) = inner.chain.blocks().get(idx) else {
                return Ok(Some(vec![OP_INV_ACK]));
            };
            if b.block_hash != want_hash {
                return Ok(Some(vec![OP_INV_ACK]));
            }
            let mut msg = vec![OP_BLOCK];
            msg.extend_from_slice(&encode_block(b));
            Ok(Some(msg))
        }
        OP_INV_ACK => {
            if payload.len() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "INV_ACK body length",
                ));
            }
            Ok(None)
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unknown wire opcode",
        )),
    }
}

/// Legacy pull: no v2 handshake (mini-servers / tests).
pub fn pull_blocks_from_peer(peer: &str, start_height: u64) -> Result<Vec<Block>, String> {
    pull_blocks_from_peer_inner(peer, start_height, None, 0, "")
}

/// When `wire` is `Some` and `handshake_outbound` is set, performs HELLO first.
pub fn pull_blocks_from_peer_inner(
    peer: &str,
    start_height: u64,
    wire: Option<&WireRuntimeConfig>,
    tip_height: u64,
    tip_hash: &str,
) -> Result<Vec<Block>, String> {
    let mut stream = TcpStream::connect(peer).map_err(|e| format!("connect {peer}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(60))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(60))).ok();
    if let Some(w) = wire {
        if w.handshake_outbound {
            outbound_handshake(&mut stream, w, tip_height, tip_hash)?;
        }
    }
    let req = wire_encode_get_blocks(start_height);
    write_framed(&mut stream, &req).map_err(|e| e.to_string())?;
    let resp = read_framed(&mut stream).map_err(|e| e.to_string())?;
    wire_decode_blocks_response(&resp)
}

/// Fetch blocks with height >= `chain.height() + 1` and append each in order.
///
/// Performs multiple `GET_BLOCKS` rounds until the peer returns an empty batch,
/// [`MAX_SYNC_PULL_ROUNDS`] non-empty rounds would be exceeded, or [`MAX_BLOCKS_APPLIED_PER_SYNC`]
/// (operational cap) would be exceeded (each round uses a new TCP connection when handshake is on).
pub fn sync_from_peer(inner: &mut NodeInner, peer: &str, now_unix: u64) -> Result<usize, String> {
    let max_blocks = MAX_BLOCKS_APPLIED_PER_SYNC as usize;
    let mut total = 0usize;
    let mut non_empty_pulls: u32 = 0;
    loop {
        let start = inner.chain.height().saturating_add(1);
        let (tip_height, tip_hash) = {
            let tip = inner
                .chain
                .blocks()
                .last()
                .ok_or_else(|| "missing chain tip".to_string())?;
            (tip.height, tip.block_hash.clone())
        };
        let blocks =
            pull_blocks_from_peer_inner(peer, start, Some(&inner.wire), tip_height, &tip_hash)?;
        if blocks.is_empty() {
            break;
        }
        if non_empty_pulls >= MAX_SYNC_PULL_ROUNDS {
            return Err(format!(
                "sync {peer}: exceeded {MAX_SYNC_PULL_ROUNDS} non-empty pull rounds (possible hostile or stalled peer)"
            ));
        }
        if total.saturating_add(blocks.len()) > max_blocks {
            return Err(format!(
                "sync {peer}: would exceed max blocks per sync ({max_blocks}); already applied {total}"
            ));
        }
        non_empty_pulls += 1;
        for b in blocks {
            inner.append_network_block_persist(b, now_unix)?;
            total += 1;
        }
    }
    if inner.wire.exchange_peers {
        match exchange_peers_with_peer(inner, peer) {
            Ok(m) if m > 0 => {
                diag::line("peers", format!("merged {m} new address(es) from {peer}"));
            }
            Ok(_) => {}
            Err(e) => diag::line("peers", format!("exchange with {peer} failed: {e}")),
        }
    }
    Ok(total)
}

pub fn push_block_to_peer(peer: &str, block: &Block) -> Result<(), String> {
    push_block_to_peer_inner(peer, block, None, 0, "")
}

pub fn push_block_to_peer_inner(
    peer: &str,
    block: &Block,
    wire: Option<&WireRuntimeConfig>,
    tip_height: u64,
    tip_hash: &str,
) -> Result<(), String> {
    let mut stream = TcpStream::connect(peer).map_err(|e| format!("connect {peer}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(60))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(60))).ok();
    if let Some(w) = wire {
        if w.handshake_outbound {
            outbound_handshake(&mut stream, w, tip_height, tip_hash)?;
        }
    }
    if wire.map(|w| w.announce_blocks).unwrap_or(false) {
        let inv = encode_height_hash_wire(OP_BLOCK_INV, block.height, &block.block_hash)
            .map_err(|e| e.to_string())?;
        write_framed(&mut stream, &inv).map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        let resp = read_framed(&mut stream).map_err(|e| e.to_string())?;
        match resp.first() {
            Some(&OP_BLOCK_WANT) => {
                let (h, hash) = decode_height_hash_body(&resp[1..]).map_err(|e| e.to_string())?;
                if h != block.height || hash != block.block_hash {
                    return Err("BLOCK_WANT height/hash mismatch".into());
                }
                let mut msg = vec![OP_BLOCK];
                msg.extend_from_slice(&encode_block(block));
                write_framed(&mut stream, &msg).map_err(|e| e.to_string())?;
            }
            Some(&OP_INV_ACK) => {}
            Some(o) => {
                return Err(format!(
                    "after BLOCK_INV expected BLOCK_WANT or INV_ACK, got opcode {o}"
                ));
            }
            None => return Err("empty response after BLOCK_INV".into()),
        }
        return Ok(());
    }
    let mut msg = vec![OP_BLOCK];
    msg.extend_from_slice(&encode_block(block));
    write_framed(&mut stream, &msg).map_err(|e| e.to_string())
}

pub fn push_tx_to_peer(peer: &str, tx: &Transaction) -> Result<(), String> {
    push_tx_to_peer_inner(peer, tx, None, 0, "")
}

pub fn push_tx_to_peer_inner(
    peer: &str,
    tx: &Transaction,
    wire: Option<&WireRuntimeConfig>,
    tip_height: u64,
    tip_hash: &str,
) -> Result<(), String> {
    let mut stream = TcpStream::connect(peer).map_err(|e| format!("connect {peer}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(60))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(60))).ok();
    if let Some(w) = wire {
        if w.handshake_outbound {
            outbound_handshake(&mut stream, w, tip_height, tip_hash)?;
        }
    }
    let mut msg = vec![OP_TX];
    msg.extend_from_slice(&encode_transaction(tx));
    write_framed(&mut stream, &msg).map_err(|e| e.to_string())
}

fn write_reject(stream: &mut TcpStream, msg: &str) -> io::Result<()> {
    let mut payload = vec![OP_REJECT];
    let b = msg.as_bytes();
    let take = b.len().min(512);
    payload.extend_from_slice(&(take as u16).to_be_bytes());
    payload.extend_from_slice(&b[..take]);
    write_framed(stream, &payload)
}

/// Handle framed messages on one connection until EOF.
pub fn peer_connection_loop(stream: TcpStream, state: Arc<Mutex<NodeInner>>) -> io::Result<()> {
    let remote = stream.peer_addr()?;
    let remote_label = remote.to_string();
    let (require_hs, allow_legacy) = {
        let g = state.lock().expect("node lock poisoned");
        (
            g.wire.require_handshake_inbound,
            g.wire.allow_legacy_inbound,
        )
    };
    let mut stream = stream;
    stream.set_read_timeout(Some(Duration::from_secs(120))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(60))).ok();

    let mut first = true;
    let mut frames_seen: u32 = 0;
    loop {
        frames_seen = frames_seen.saturating_add(1);
        if frames_seen > MAX_FRAMES_PER_INBOUND_SESSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "inbound framed message limit exceeded",
            ));
        }
        let frame = match read_framed(&mut stream) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        };
        let now = unix_now_secs();
        let response = if first {
            first = false;
            let mut g = state.lock().expect("node lock poisoned");
            match frame.first() {
                Some(&OP_HELLO) => {
                    if let Err(e) = validate_remote_hello_against_local(&g, &frame[1..]) {
                        diag::line("handshake", format!("reject: {e}"));
                        let _ = write_reject(&mut stream, &e);
                        return Err(io::Error::other(e));
                    }
                    g.peer_book.merge_inbound_peer(remote_label.clone());
                    Some(build_hello_ok(&g).map_err(io::Error::other)?)
                }
                Some(_) if require_hs => {
                    diag::line("handshake", "required HELLO as first frame");
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "HELLO required as first frame",
                    ));
                }
                Some(_) if allow_legacy => {
                    process_payload(&frame, &mut g, now, Some(remote_label.as_str()))?
                }
                Some(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "HELLO required as first frame",
                    ));
                }
                None => {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "empty frame"));
                }
            }
        } else {
            let mut g = state.lock().expect("node lock poisoned");
            process_payload(&frame, &mut g, now, Some(remote_label.as_str()))?
        };
        if let Some(resp) = response {
            write_framed(&mut stream, &resp)?;
        }
    }
    Ok(())
}

pub fn serve_tcp_listener(listener: TcpListener, state: Arc<Mutex<NodeInner>>) {
    loop {
        match listener.accept() {
            Ok((stream, addr)) => {
                let st = state.clone();
                thread::spawn(move || {
                    if let Err(e) = peer_connection_loop(stream, st) {
                        diag::line("network", format!("session {addr} ended: {e}"));
                    }
                });
            }
            Err(e) => diag::line("network", format!("accept error: {e}")),
        }
    }
}

/// Bind and spawn the accept loop. Returns the resolved address (e.g. with port when using `:0`).
pub fn spawn_incoming_loop(
    bind: &str,
    state: Arc<Mutex<NodeInner>>,
) -> io::Result<(thread::JoinHandle<()>, String)> {
    let listener = TcpListener::bind(bind)?;
    let addr = listener.local_addr()?.to_string();
    let handle = thread::spawn(move || serve_tcp_listener(listener, state));
    Ok((handle, addr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::crypto::Crypto;
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
    fn wire_decode_blocks_rejects_excessive_batch_count() {
        let mut payload = vec![OP_BLOCKS];
        let bad_count = MAX_BLOCKS_PER_WIRE_BATCH.saturating_add(1);
        payload.extend_from_slice(&bad_count.to_be_bytes());
        let err = wire_decode_blocks_response(&payload).unwrap_err();
        assert!(err.contains("batch count"), "unexpected: {err}");
    }

    #[test]
    fn wire_decode_blocks_rejects_oversized_block_length_field() {
        let b = one_valid_block();
        let enc = encode_block(&b);
        let mut payload = vec![OP_BLOCKS];
        payload.extend_from_slice(&1u32.to_be_bytes());
        let over = MAX_WIRE_FRAME_BYTES.saturating_add(1);
        payload.extend_from_slice(&over.to_be_bytes());
        payload.extend_from_slice(&enc);
        let err = wire_decode_blocks_response(&payload).unwrap_err();
        assert!(
            err.contains("max frame") || err.contains("exceeds max"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn wire_peers_roundtrip() {
        use crate::peer_book::PeerBook;

        let mut book = PeerBook::default();
        book.merge_seed("127.0.0.1:8".into());
        book.merge_seed("127.0.0.1:9".into());
        let enc = super::wire_encode_peers(&book);
        let out = super::wire_decode_peers_response(&enc).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&"127.0.0.1:8".into()));
        assert!(out.contains(&"127.0.0.1:9".into()));
    }

    #[test]
    fn wire_peers_rejects_excessive_count() {
        let mut enc = vec![OP_PEERS];
        enc.extend_from_slice(&(MAX_PEERS_PER_WIRE_FRAME.saturating_add(1)).to_be_bytes());
        assert!(super::wire_decode_peers_response(&enc).is_err());
    }

    #[test]
    fn max_blocks_per_sync_within_wire_theoretical_max() {
        let max_by_wire =
            (MAX_SYNC_PULL_ROUNDS as u64).saturating_mul(MAX_BLOCKS_PER_WIRE_BATCH as u64);
        assert_eq!(
            MAX_BLOCKS_APPLIED_PER_SYNC_WIRE_MAX as u64, max_by_wire,
            "wire max should be rounds × max batch"
        );
        assert!(
            (MAX_BLOCKS_APPLIED_PER_SYNC as u64) <= max_by_wire,
            "operational per-sync cap must not exceed wire theoretical max"
        );
    }

    #[test]
    fn block_inv_elicits_want_for_next_height() {
        use crate::blockchain::Blockchain;
        use crate::genesis::Genesis;
        use crate::mempool::Mempool;
        use crate::storage::BlockStore;

        let path = std::env::temp_dir().join(format!(
            "trilogicon_inv_{}_{}.blocks",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);

        let mut inner = NodeInner::for_tests(
            Genesis::empty(),
            Blockchain::from_genesis(&Genesis::empty()).unwrap(),
            Mempool::new(4),
            BlockStore::open_append(&path).unwrap(),
        )
        .unwrap();

        let b = one_valid_block();
        inner
            .chain
            .state_mut()
            .create_account(b.transactions[0].sender.clone(), 1000);

        let inv = super::encode_height_hash_wire(OP_BLOCK_INV, b.height, &b.block_hash).unwrap();
        let out = super::process_payload(&inv, &mut inner, 1_700_200_000, Some("unit_test"))
            .unwrap()
            .expect("expected WANT response");
        assert_eq!(out.first().copied(), Some(OP_BLOCK_WANT));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hello_roundtrip_matches_genesis() {
        use crate::blockchain::Blockchain;
        use crate::genesis::Genesis;

        let genesis = Genesis::empty();
        let wire = WireRuntimeConfig::from_genesis(&genesis, 7, false).unwrap();
        let chain = Blockchain::from_genesis(&genesis).unwrap();
        let tip = chain.blocks().last().unwrap();
        let body = encode_hello_body(
            wire.wire_version,
            wire.network_id,
            &wire.genesis_commitment,
            tip.height,
            &tip.block_hash,
        )
        .unwrap();
        let (v, n, g, h, hash) = decode_hello_body(&body).unwrap();
        assert_eq!(v, TRIL_WIRE_VERSION);
        assert_eq!(n, 7);
        assert_eq!(g, wire.genesis_commitment);
        assert_eq!(h, 0);
        assert_eq!(hash, "GENESIS_HASH");
    }

    #[test]
    fn append_network_block_persist_rolls_back_when_persist_fails() {
        use crate::blockchain::Blockchain;
        use crate::genesis::Genesis;
        use crate::mempool::Mempool;
        use crate::storage::{BlockStore, test_set_inject_append_fail};

        let path = std::env::temp_dir().join(format!(
            "trilogicon_net_persist_{}_{}.blocks",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);

        let mut chain = Blockchain::from_genesis(&Genesis::empty()).unwrap();
        let b1 = one_valid_block();
        chain
            .state_mut()
            .create_account(b1.transactions[0].sender.clone(), 1000);

        let mut inner = NodeInner::for_tests(
            Genesis::empty(),
            chain,
            Mempool::new(4),
            BlockStore::open_append(&path).unwrap(),
        )
        .unwrap();

        test_set_inject_append_fail(true);
        let r = inner.append_network_block_persist(b1.clone(), 1_700_200_000);
        assert!(r.is_err(), "expected err, got {r:?}");
        assert_eq!(inner.chain.height(), 0);
        test_set_inject_append_fail(false);

        inner
            .chain
            .state_mut()
            .create_account(b1.transactions[0].sender.clone(), 1000);

        inner
            .append_network_block_persist(b1, 1_700_200_000)
            .unwrap();
        assert_eq!(inner.chain.height(), 1);

        let _ = std::fs::remove_file(&path);
    }
}
