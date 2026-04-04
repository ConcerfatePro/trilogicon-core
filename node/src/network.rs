//! V1 TCP peer protocol: length-prefixed frames, same canonical encodings as disk ([`crate::encoding`]).
//!
//! Opcodes (first payload byte):
//! - `1` — transaction body = [`encode_transaction`]
//! - `2` — block body = [`encode_block`]
//! - `3` — get blocks: next 8 bytes = `u64_be` `start_height` (reply is opcode `4`)
//! - `4` — block batch: `u32_be` count, then count × (`u32_be` len + block bytes)

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::block::Block;
use crate::blockchain::Blockchain;
use crate::encoding::{decode_block, decode_transaction, encode_block, encode_transaction};
use crate::mempool::Mempool;
use crate::storage::BlockStore;
use crate::transaction::Transaction;

pub const MAX_WIRE_FRAME_BYTES: u32 = 64 * 1024 * 1024;

const OP_TX: u8 = 1;
const OP_BLOCK: u8 = 2;
const OP_GET_BLOCKS: u8 = 3;
const OP_BLOCKS: u8 = 4;

/// Shared node state for the TCP listener and the block-production loop.
pub struct NodeInner {
    pub chain: Blockchain,
    pub pool: Mempool,
    pub store: BlockStore,
}

impl NodeInner {
    pub fn append_network_block_persist(
        &mut self,
        block: Block,
        now_unix: u64,
    ) -> Result<(), String> {
        self.chain
            .try_append_network_block(block, now_unix)
            .map_err(|e| e.to_string())?;
        let tip = self
            .chain
            .blocks()
            .last()
            .ok_or_else(|| "missing tip after append".to_string())?;
        self.store.append_block(tip).map_err(|e| e.to_string())
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
    let n = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
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
        Some(&OP_BLOCKS) => parse_op_blocks_body(&payload[1..]),
        Some(o) => Err(format!("expected OP_BLOCKS, got opcode {o}")),
        None => Err("empty response".into()),
    }
}

fn process_payload(
    payload: &[u8],
    inner: &mut NodeInner,
    now_unix: u64,
) -> io::Result<Option<Vec<u8>>> {
    let op = *payload
        .first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty payload"))?;

    match op {
        OP_TX => {
            let tx = decode_transaction(&payload[1..])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            if let Err(e) = inner.pool.try_submit(tx) {
                eprintln!("network: mempool rejected tx ({e})");
            }
            Ok(None)
        }
        OP_BLOCK => {
            let block = decode_block(&payload[1..])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            if let Err(e) = inner.append_network_block_persist(block, now_unix) {
                eprintln!("network: rejected block ({e})");
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
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unknown wire opcode",
        )),
    }
}

/// Handle framed messages on one connection until EOF.
pub fn peer_connection_loop(stream: TcpStream, state: Arc<Mutex<NodeInner>>) -> io::Result<()> {
    let mut stream = stream;
    stream.set_read_timeout(Some(Duration::from_secs(120))).ok();

    loop {
        let frame = match read_framed(&mut stream) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        };
        let now = unix_now_secs();
        let response = {
            let mut g = state.lock().expect("node lock poisoned");
            process_payload(&frame, &mut g, now)?
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
                        eprintln!("network: session {addr}: {e}");
                    }
                });
            }
            Err(e) => eprintln!("network: accept: {e}"),
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

pub fn pull_blocks_from_peer(peer: &str, start_height: u64) -> Result<Vec<Block>, String> {
    let mut stream = TcpStream::connect(peer).map_err(|e| format!("connect {peer}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(60))).ok();
    let req = wire_encode_get_blocks(start_height);
    write_framed(&mut stream, &req).map_err(|e| e.to_string())?;
    let resp = read_framed(&mut stream).map_err(|e| e.to_string())?;
    wire_decode_blocks_response(&resp)
}

/// Fetch blocks with height >= `chain.height() + 1` and append each in order.
pub fn sync_from_peer(inner: &mut NodeInner, peer: &str, now_unix: u64) -> Result<usize, String> {
    let start = inner.chain.height().saturating_add(1);
    let blocks = pull_blocks_from_peer(peer, start)?;
    let mut n = 0usize;
    for b in blocks {
        inner.append_network_block_persist(b, now_unix)?;
        n += 1;
    }
    Ok(n)
}

pub fn push_block_to_peer(peer: &str, block: &Block) -> Result<(), String> {
    let mut stream = TcpStream::connect(peer).map_err(|e| format!("connect {peer}: {e}"))?;
    let mut msg = vec![OP_BLOCK];
    msg.extend_from_slice(&encode_block(block));
    write_framed(&mut stream, &msg).map_err(|e| e.to_string())
}

pub fn push_tx_to_peer(peer: &str, tx: &Transaction) -> Result<(), String> {
    let mut stream = TcpStream::connect(peer).map_err(|e| format!("connect {peer}: {e}"))?;
    let mut msg = vec![OP_TX];
    msg.extend_from_slice(&encode_transaction(tx));
    write_framed(&mut stream, &msg).map_err(|e| e.to_string())
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
}
