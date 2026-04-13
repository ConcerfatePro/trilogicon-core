//! Append-only block file. Two on-disk layouts (both are **local storage** only; block bytes are
//! still canonical [`encode_block`] payloads):
//!
//! - **Legacy:** repeated `u32_be frame_len` + `encode_block` payload (V1-era nodes).
//! - **V2:** optional `CHAIN_FILE_MAGIC_V2` header on fresh files, then per record:
//!   `u32_be frame_len` + payload + `u32_be crc32_ieee(payload)` to detect torn/partial frames.
//!
//! Genesis **block** is not stored; replay applies stored blocks on top of
//! [`Blockchain::from_genesis`] using the same protocol [`Genesis`](crate::genesis::Genesis) document.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::block::Block;
use crate::blockchain::Blockchain;
use crate::encoding::{EncodeError, decode_block, encode_block};
use crate::errors::ProtocolError;
use crate::genesis::Genesis;

/// Hard cap per frame to limit hostile allocations when reading untrusted files.
const MAX_FRAME_BYTES: u32 = 64 * 1024 * 1024;

/// Marks V2 `chain.blocks`: 8-byte magic at file start; new empty files use this format on first append.
const CHAIN_FILE_MAGIC_V2: &[u8; 8] = b"TRILBC01";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChainFileFormat {
    Legacy,
    V2,
}

/// IEEE 802.3 CRC-32 over `payload` only (detects truncated writes and single-bit corruption).
fn crc32_ieee(payload: &[u8]) -> u32 {
    let mut c: u32 = 0xFFFF_FFFF;
    for byte in payload {
        c ^= u32::from(*byte);
        for _ in 0..8 {
            if c & 1 != 0 {
                c = (c >> 1) ^ 0xEDB8_8320;
            } else {
                c >>= 1;
            }
        }
    }
    !c
}

#[derive(Debug)]
pub enum StorageError {
    Io(io::Error),
    Decode(String),
    Replay(ProtocolError),
    Genesis(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Io(e) => write!(f, "storage I/O: {e}"),
            StorageError::Decode(s) => write!(f, "storage decode: {s}"),
            StorageError::Replay(e) => write!(f, "chain replay: {e}"),
            StorageError::Genesis(s) => write!(f, "genesis: {s}"),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for StorageError {
    fn from(e: io::Error) -> Self {
        StorageError::Io(e)
    }
}

impl From<EncodeError> for StorageError {
    fn from(e: EncodeError) -> Self {
        StorageError::Decode(e.0)
    }
}

/// Open a file for appending length-prefixed block frames.
pub struct BlockStore {
    file: File,
    /// After any failed append/sync, further writes are refused: on-disk `chain.blocks` may be
    /// truncated or partially written; continuing would risk diverging RAM from durable state.
    poisoned: bool,
    format: ChainFileFormat,
    /// When [`ChainFileFormat::V2`] and the file was empty at open, magic is written on first append.
    v2_magic_on_disk: bool,
}

impl BlockStore {
    pub fn open_append(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)?;
        let len = file.metadata()?.len();
        let (format, v2_magic_on_disk) = match len {
            0 => (ChainFileFormat::V2, false),
            n if n < CHAIN_FILE_MAGIC_V2.len() as u64 => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{} chain.blocks: file too short ({n} bytes) — truncated or corrupt",
                        crate::operator_msg::PFX_STORAGE
                    ),
                ));
            }
            _ => {
                file.seek(SeekFrom::Start(0))?;
                let mut hdr = [0u8; 8];
                file.read_exact(&mut hdr)?;
                let fmt = if hdr == *CHAIN_FILE_MAGIC_V2 {
                    (ChainFileFormat::V2, true)
                } else {
                    (ChainFileFormat::Legacy, false)
                };
                file.seek(SeekFrom::End(0))?;
                fmt
            }
        };
        Ok(Self {
            file,
            poisoned: false,
            format,
            v2_magic_on_disk,
        })
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    #[cfg(test)]
    pub(crate) fn mark_poisoned_for_tests(&mut self) {
        self.poisoned = true;
    }

    /// Write one frame. Legacy: `u32_be len` + payload. V2: same plus trailing CRC-32 of payload.
    /// Uses a single `write_all` per logical frame (after optional one-time magic write) to reduce
    /// torn-record risk vs separate length/payload syscalls.
    pub fn append_payload(&mut self, payload: &[u8]) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::other(format!(
                "{} fail-closed: refusing chain.blocks append after a prior write/sync failure (on-disk state may be inconsistent — stop and repair or restore chain.blocks)",
                crate::operator_msg::PFX_STORAGE
            )));
        }
        let len = u32::try_from(payload.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "encoded block exceeds u32 frame length",
            )
        })?;
        if len > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "encoded block exceeds MAX_FRAME_BYTES",
            ));
        }

        let res = (|| {
            if matches!(self.format, ChainFileFormat::V2) && !self.v2_magic_on_disk {
                self.file.write_all(CHAIN_FILE_MAGIC_V2)?;
                self.file.sync_all()?;
                self.v2_magic_on_disk = true;
            }

            match self.format {
                ChainFileFormat::Legacy => {
                    let mut buf = Vec::with_capacity(4 + payload.len());
                    buf.extend_from_slice(&len.to_be_bytes());
                    buf.extend_from_slice(payload);
                    self.file.write_all(&buf)?;
                }
                ChainFileFormat::V2 => {
                    let crc = crc32_ieee(payload);
                    let mut buf = Vec::with_capacity(4 + payload.len() + 4);
                    buf.extend_from_slice(&len.to_be_bytes());
                    buf.extend_from_slice(payload);
                    buf.extend_from_slice(&crc.to_be_bytes());
                    self.file.write_all(&buf)?;
                }
            }
            self.file.sync_all()
        })();
        if res.is_err() {
            self.poisoned = true;
        }
        res
    }

    pub fn append_block(&mut self, block: &Block) -> io::Result<()> {
        self.append_payload(&encode_block(block))
    }

    /// Read all blocks in file order (missing file → empty vec).
    pub fn read_all_blocks(path: impl AsRef<Path>) -> Result<Vec<Block>, StorageError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let data = fs::read(path)?;
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let (format, body) = if data.len() >= CHAIN_FILE_MAGIC_V2.len()
            && data[..CHAIN_FILE_MAGIC_V2.len()] == *CHAIN_FILE_MAGIC_V2
        {
            (ChainFileFormat::V2, &data[CHAIN_FILE_MAGIC_V2.len()..])
        } else {
            (ChainFileFormat::Legacy, data.as_slice())
        };

        match format {
            ChainFileFormat::Legacy => parse_legacy_block_frames(body),
            ChainFileFormat::V2 => parse_v2_block_frames(body),
        }
    }

    /// Like [`Self::read_all_blocks`], but if the file ends with 1–3 trailing bytes after the last
    /// complete frame (incomplete length prefix), truncates them and returns `repaired = true`.
    /// A full length prefix for an incomplete next frame returns [`StorageError::Decode`].
    pub fn read_all_blocks_repairing_tail(
        path: impl AsRef<Path>,
    ) -> Result<(Vec<Block>, bool), StorageError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok((Vec::new(), false));
        }
        let mut data = fs::read(path)?;
        if data.is_empty() {
            return Ok((Vec::new(), false));
        }
        let (format, body_off) = if data.len() >= CHAIN_FILE_MAGIC_V2.len()
            && data[..CHAIN_FILE_MAGIC_V2.len()] == *CHAIN_FILE_MAGIC_V2
        {
            (ChainFileFormat::V2, CHAIN_FILE_MAGIC_V2.len())
        } else {
            (ChainFileFormat::Legacy, 0usize)
        };
        let body = &data[body_off..];
        let (blocks, consumed_in_body) = match format {
            ChainFileFormat::Legacy => parse_legacy_block_frames_prefix(body)?,
            ChainFileFormat::V2 => parse_v2_block_frames_prefix(body)?,
        };
        let valid_end = body_off + consumed_in_body;
        let repaired = valid_end < data.len();
        if repaired {
            if blocks.is_empty() && consumed_in_body == 0 {
                return Err(StorageError::Decode(
                    "chain.blocks corrupt or truncated (no complete block frame)".into(),
                ));
            }
            let trailing = data.len() - valid_end;
            // If at least a full u32 length prefix is present after the last good frame, we are
            // unambiguously inside a subsequent frame; refuse silent truncation (crash-safe repair
            // only covers a short, incomplete length write: < 4 bytes).
            if trailing >= 4 {
                return Err(StorageError::Decode(
                    "chain.blocks truncated after last complete block frame".into(),
                ));
            }
            data.truncate(valid_end);
            fs::write(path, &data)?;
        }
        Ok((blocks, repaired))
    }
}

/// Parse complete legacy frames from the start of `data`; stop before the first partial frame.
fn parse_legacy_block_frames_prefix(data: &[u8]) -> Result<(Vec<Block>, usize), StorageError> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let frame_start = pos;
        if pos + 4 > data.len() {
            break;
        }
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if len as u32 > MAX_FRAME_BYTES {
            return Err(StorageError::Decode("frame exceeds max size".into()));
        }
        if pos + len > data.len() {
            return Ok((out, frame_start));
        }
        let payload = &data[pos..pos + len];
        pos += len;
        let block = decode_block(payload)?;
        out.push(block);
    }
    Ok((out, pos))
}

/// Parse complete V2 frames from the start of `data` (no magic; body only); stop before partial.
fn parse_v2_block_frames_prefix(data: &[u8]) -> Result<(Vec<Block>, usize), StorageError> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let frame_start = pos;
        if pos + 4 > data.len() {
            break;
        }
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if len as u32 > MAX_FRAME_BYTES {
            return Err(StorageError::Decode("frame exceeds max size (v2)".into()));
        }
        let end_payload = pos
            .checked_add(len)
            .ok_or_else(|| StorageError::Decode("v2 frame length overflow".into()))?;
        if end_payload + 4 > data.len() {
            return Ok((out, frame_start));
        }
        let payload = &data[pos..end_payload];
        let stored_crc = u32::from_be_bytes([
            data[end_payload],
            data[end_payload + 1],
            data[end_payload + 2],
            data[end_payload + 3],
        ]);
        let expect = crc32_ieee(payload);
        if stored_crc != expect {
            return Err(StorageError::Decode(format!(
                "v2 frame crc mismatch (stored {stored_crc:#x} != expected {expect:#x})"
            )));
        }
        pos = end_payload + 4;
        let block = decode_block(payload)?;
        out.push(block);
    }
    Ok((out, pos))
}

fn parse_legacy_block_frames(data: &[u8]) -> Result<Vec<Block>, StorageError> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        if pos + 4 > data.len() {
            return Err(StorageError::Decode("truncated frame length".into()));
        }
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if len as u32 > MAX_FRAME_BYTES {
            return Err(StorageError::Decode("frame exceeds max size".into()));
        }
        if pos + len > data.len() {
            return Err(StorageError::Decode("truncated frame body".into()));
        }
        let payload = &data[pos..pos + len];
        pos += len;
        let block = decode_block(payload)?;
        out.push(block);
    }
    Ok(out)
}

fn parse_v2_block_frames(data: &[u8]) -> Result<Vec<Block>, StorageError> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        if pos + 4 > data.len() {
            return Err(StorageError::Decode("truncated frame length (v2)".into()));
        }
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if len as u32 > MAX_FRAME_BYTES {
            return Err(StorageError::Decode("frame exceeds max size (v2)".into()));
        }
        let end_payload = pos
            .checked_add(len)
            .ok_or_else(|| StorageError::Decode("v2 frame length overflow".into()))?;
        if end_payload + 4 > data.len() {
            return Err(StorageError::Decode(
                "truncated v2 frame (crc or body)".into(),
            ));
        }
        let payload = &data[pos..end_payload];
        let stored_crc = u32::from_be_bytes([
            data[end_payload],
            data[end_payload + 1],
            data[end_payload + 2],
            data[end_payload + 3],
        ]);
        pos = end_payload + 4;
        let expect = crc32_ieee(payload);
        if stored_crc != expect {
            return Err(StorageError::Decode(format!(
                "v2 frame crc mismatch (stored {stored_crc:#x} != expected {expect:#x})"
            )));
        }
        let block = decode_block(payload)?;
        out.push(block);
    }
    Ok(out)
}

fn chain_from_genesis(genesis: &Genesis) -> Result<Blockchain, StorageError> {
    Blockchain::from_genesis(genesis).map_err(|e| match e {
        ProtocolError::GenesisError(s) => StorageError::Genesis(s),
        other => StorageError::Replay(other),
    })
}

/// Build chain from `genesis` (height-0 state), then apply every stored block in file order.
/// Empty or missing file → chain is genesis tip only.
///
/// If the file ends with fewer than four trailing bytes after the last complete block (incomplete
/// length prefix only), those bytes are truncated and the second return value is `true`. A full
/// length prefix for a partial next frame is rejected (fail closed).
pub fn load_blockchain_from_disk(
    path: impl AsRef<Path>,
    genesis: &Genesis,
) -> Result<(Blockchain, bool), StorageError> {
    let (blocks, repaired) = BlockStore::read_all_blocks_repairing_tail(path)?;
    let mut chain = chain_from_genesis(genesis)?;
    for block in blocks {
        chain.append_block(block).map_err(StorageError::Replay)?;
    }
    Ok((chain, repaired))
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;

    use super::*;
    use crate::crypto::Crypto;
    use crate::genesis::{Genesis, GenesisAllocation};
    use crate::transaction::Transaction;
    use crate::types::Address;
    use ed25519_dalek::{Signer, SigningKey};

    fn unique_store_path(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "trilogicon_store_{}_{}_{}.blocks",
            label,
            std::process::id(),
            nanos
        ))
    }

    fn signed_tx(
        seed: u8,
        receiver: &str,
        amount: u64,
        fee: u64,
        nonce: u64,
        ts: u64,
    ) -> Transaction {
        let signing_key = SigningKey::from_bytes(&[seed; 32]);
        let verifying_key = signing_key.verifying_key();
        let mut tx = Transaction {
            sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
            receiver: Address::new(receiver),
            amount,
            fee,
            nonce,
            timestamp_unix: ts,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let p = tx.unsigned_payload_bytes();
        tx.signature = signing_key.sign(&p).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&p);
        tx
    }

    fn seal(mut b: Block) -> Block {
        b.block_hash = b.compute_block_hash();
        b
    }

    /// Test-only helper: extends RAM then persists. Production paths must not extend the chain
    /// before durable append succeeds (see V2 persistence note).
    fn append_block_and_persist(
        chain: &mut Blockchain,
        store: &mut BlockStore,
        block: Block,
    ) -> Result<(), StorageError> {
        let payload = encode_block(&block);
        chain.append_block(block).map_err(StorageError::Replay)?;
        store.append_payload(&payload)?;
        Ok(())
    }

    #[test]
    fn append_and_load_roundtrip_with_replay() {
        let path = unique_store_path("roundtrip");
        let _ = std::fs::remove_file(&path);

        let signing_key = SigningKey::from_bytes(&[60u8; 32]);
        let vk = signing_key.verifying_key();
        let sender = Address::new(Crypto::address_from_public_key(&vk.to_bytes()));
        let genesis = Genesis {
            allocations: vec![GenesisAllocation {
                address: sender.0.clone(),
                balance: 100,
            }],
        };

        {
            let mut chain = Blockchain::from_genesis(&genesis).unwrap();
            let mut store = BlockStore::open_append(&path).unwrap();

            let tx = signed_tx(60, "recv_store", 10, 1, 0, 1);
            let b1 = seal(Block {
                height: 1,
                previous_hash: "GENESIS_HASH".into(),
                timestamp_unix: 100,
                transactions: vec![tx],
                block_hash: String::new(),
            });
            append_block_and_persist(&mut chain, &mut store, b1).unwrap();

            let tx2 = signed_tx(60, "recv_store", 5, 1, 1, 2);
            let tip_hash = chain.blocks().last().unwrap().block_hash.clone();
            let b2 = seal(Block {
                height: 2,
                previous_hash: tip_hash,
                timestamp_unix: 101,
                transactions: vec![tx2],
                block_hash: String::new(),
            });
            append_block_and_persist(&mut chain, &mut store, b2).unwrap();

            let s = chain.state().get_account(&sender).unwrap();
            assert_eq!(s.balance, 83);
            assert_eq!(s.nonce, 2);
        }

        let (loaded, _) = load_blockchain_from_disk(&path, &genesis).unwrap();
        let s = loaded.state().get_account(&sender).unwrap();
        assert_eq!(s.balance, 83);
        assert_eq!(s.nonce, 2);
        assert_eq!(loaded.height(), 2);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_loads_empty_chain() {
        let path = unique_store_path("missing");
        let _ = std::fs::remove_file(&path);
        let (chain, _) = load_blockchain_from_disk(&path, &Genesis::empty()).unwrap();
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn truncated_length_prefix_without_complete_frame_fails_closed() {
        let path = unique_store_path("trunc");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, [0u8, 0u8, 0u8, 0x10]).unwrap();
        let err = match load_blockchain_from_disk(&path, &Genesis::empty()) {
            Err(e) => e,
            Ok(_) => panic!("expected corrupt chain file to fail load"),
        };
        let s = err.to_string();
        assert!(
            s.contains("corrupt") || s.contains("truncated"),
            "unexpected err: {s}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_all_blocks_rejects_oversized_frame_length() {
        let path = unique_store_path("biglen");
        let _ = std::fs::remove_file(&path);
        let over = MAX_FRAME_BYTES.saturating_add(1);
        std::fs::write(&path, over.to_be_bytes()).unwrap();
        let err = BlockStore::read_all_blocks(&path).unwrap_err();
        assert!(
            matches!(err, StorageError::Decode(ref s) if s.contains("max size")),
            "unexpected err: {err}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_all_blocks_rejects_decode_error_inside_frame() {
        let path = unique_store_path("badpayload");
        let _ = std::fs::remove_file(&path);
        let payload = vec![0xABu8; 64];
        let len = u32::try_from(payload.len()).unwrap();
        let mut file = len.to_be_bytes().to_vec();
        file.extend_from_slice(&payload);
        std::fs::write(&path, &file).unwrap();
        let err = BlockStore::read_all_blocks(&path).unwrap_err();
        assert!(
            matches!(err, StorageError::Decode(_)),
            "expected Decode, got {err:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_blockchain_from_disk_rejects_invalid_second_block_on_replay() {
        let path = unique_store_path("replay_fail");
        let _ = std::fs::remove_file(&path);

        let signing_key = SigningKey::from_bytes(&[61u8; 32]);
        let vk = signing_key.verifying_key();
        let sender = Address::new(Crypto::address_from_public_key(&vk.to_bytes()));
        let genesis = Genesis {
            allocations: vec![GenesisAllocation {
                address: sender.0.clone(),
                balance: 100,
            }],
        };

        let mut store = BlockStore::open_append(&path).unwrap();
        let tx = signed_tx(61, "recv_bad_replay", 5, 1, 0, 1);
        let b1 = seal(Block {
            height: 1,
            previous_hash: "GENESIS_HASH".into(),
            timestamp_unix: 200,
            transactions: vec![tx],
            block_hash: String::new(),
        });
        store.append_block(&b1).unwrap();

        let tx2 = signed_tx(61, "recv_bad_replay", 5, 1, 1, 2);
        let b2_bad = seal(Block {
            height: 2,
            previous_hash: "NOT_THE_TIP".into(),
            timestamp_unix: 201,
            transactions: vec![tx2],
            block_hash: String::new(),
        });
        store.append_block(&b2_bad).unwrap();
        drop(store);

        match load_blockchain_from_disk(&path, &genesis) {
            Ok(_) => panic!("expected replay error"),
            Err(e) => assert!(
                matches!(e, StorageError::Replay(ProtocolError::InvalidBlock(_))),
                "unexpected err: {e}"
            ),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn poisoned_store_refuses_further_appends() {
        let path = unique_store_path("poisoned_refuse");
        let _ = std::fs::remove_file(&path);
        let mut store = BlockStore::open_append(&path).unwrap();
        store.mark_poisoned_for_tests();
        let tx = signed_tx(63, "rpoi", 1, 1, 0, 1);
        let b = seal(Block {
            height: 1,
            previous_hash: "GENESIS_HASH".into(),
            timestamp_unix: 400,
            transactions: vec![tx],
            block_hash: String::new(),
        });
        assert!(store.append_block(&b).is_err());
        assert!(store.is_poisoned());
        let _ = std::fs::remove_file(&path);
    }

    /// Poison is **in-RAM only** (not written to `chain.blocks`). A new process — or a new
    /// `BlockStore` after drop — does not inherit poison; if the file on disk is still valid,
    /// appends can resume (restart does not clear partial writes; see persistence design note).
    #[test]
    fn poison_is_process_local_fresh_blockstore_appends_same_file() {
        let path = unique_store_path("poison_reopen");
        let _ = std::fs::remove_file(&path);

        let tx1 = signed_tx(64, "rpoi2", 1, 1, 0, 1);
        let b1 = seal(Block {
            height: 1,
            previous_hash: "GENESIS_HASH".into(),
            timestamp_unix: 500,
            transactions: vec![tx1],
            block_hash: String::new(),
        });
        let mut store = BlockStore::open_append(&path).unwrap();
        store.append_block(&b1).unwrap();
        store.mark_poisoned_for_tests();

        let tx2 = signed_tx(64, "rpoi2", 1, 1, 1, 2);
        let b2 = seal(Block {
            height: 2,
            previous_hash: b1.block_hash.clone(),
            timestamp_unix: 501,
            transactions: vec![tx2],
            block_hash: String::new(),
        });
        assert!(store.append_block(&b2).is_err());
        drop(store);

        let mut store2 = BlockStore::open_append(&path).unwrap();
        assert!(!store2.is_poisoned());
        store2.append_block(&b2).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn crc32_ieee_matches_standard_test_vector() {
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn open_append_rejects_too_short_chain_file() {
        let path = unique_store_path("short_hdr");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, [1u8, 2, 3]).unwrap();
        let err = match BlockStore::open_append(&path) {
            Ok(_) => panic!("expected short chain file to be rejected"),
            Err(e) => e,
        };
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn v2_format_roundtrip_with_crc() {
        let path = unique_store_path("v2crc");
        let _ = std::fs::remove_file(&path);

        let signing_key = SigningKey::from_bytes(&[65u8; 32]);
        let vk = signing_key.verifying_key();
        let sender = Address::new(Crypto::address_from_public_key(&vk.to_bytes()));
        let genesis = Genesis {
            allocations: vec![GenesisAllocation {
                address: sender.0.clone(),
                balance: 100,
            }],
        };

        {
            let mut chain = Blockchain::from_genesis(&genesis).unwrap();
            let mut store = BlockStore::open_append(&path).unwrap();
            let tx = signed_tx(65, "v2r", 10, 1, 0, 1);
            let b1 = seal(Block {
                height: 1,
                previous_hash: "GENESIS_HASH".into(),
                timestamp_unix: 100,
                transactions: vec![tx],
                block_hash: String::new(),
            });
            append_block_and_persist(&mut chain, &mut store, b1).unwrap();
            let raw = std::fs::read(&path).unwrap();
            assert!(
                raw.starts_with(CHAIN_FILE_MAGIC_V2),
                "new store should write v2 magic"
            );
        }

        let loaded = load_blockchain_from_disk(&path, &genesis).unwrap();
        assert_eq!(loaded.0.height(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn v2_crc_mismatch_fails_load() {
        let path = unique_store_path("v2badcrc");
        let _ = std::fs::remove_file(&path);

        let signing_key = SigningKey::from_bytes(&[66u8; 32]);
        let vk = signing_key.verifying_key();
        let sender = Address::new(Crypto::address_from_public_key(&vk.to_bytes()));
        let genesis = Genesis {
            allocations: vec![GenesisAllocation {
                address: sender.0.clone(),
                balance: 100,
            }],
        };

        {
            let mut chain = Blockchain::from_genesis(&genesis).unwrap();
            let mut store = BlockStore::open_append(&path).unwrap();
            let tx = signed_tx(66, "badcrc", 1, 1, 0, 1);
            let b1 = seal(Block {
                height: 1,
                previous_hash: "GENESIS_HASH".into(),
                timestamp_unix: 100,
                transactions: vec![tx],
                block_hash: String::new(),
            });
            append_block_and_persist(&mut chain, &mut store, b1).unwrap();
        }

        let mut raw = std::fs::read(&path).unwrap();
        assert!(raw.len() > 4);
        *raw.last_mut().expect("crc byte") ^= 0xFF;
        std::fs::write(&path, &raw).unwrap();

        let err = match load_blockchain_from_disk(&path, &genesis) {
            Ok(_) => panic!("expected crc mismatch load failure"),
            Err(e) => e,
        };
        assert!(
            matches!(&err, StorageError::Decode(s) if s.contains("crc mismatch")),
            "unexpected err: {err}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_all_blocks_rejects_trailing_byte_after_one_frame() {
        let path = unique_store_path("trail1");
        let _ = std::fs::remove_file(&path);

        let signing_key = SigningKey::from_bytes(&[62u8; 32]);
        let vk = signing_key.verifying_key();
        let sender = Address::new(Crypto::address_from_public_key(&vk.to_bytes()));
        let genesis = Genesis {
            allocations: vec![GenesisAllocation {
                address: sender.0.clone(),
                balance: 100,
            }],
        };

        let mut store = BlockStore::open_append(&path).unwrap();
        let tx = signed_tx(62, "rtrail", 1, 1, 0, 1);
        let b1 = seal(Block {
            height: 1,
            previous_hash: "GENESIS_HASH".into(),
            timestamp_unix: 300,
            transactions: vec![tx],
            block_hash: String::new(),
        });
        store.append_block(&b1).unwrap();
        drop(store);
        // Append one garbage byte after a valid frame (simulates truncated next length prefix).
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&[0x00]).unwrap();

        let err = BlockStore::read_all_blocks(&path).unwrap_err();
        assert!(
            matches!(&err, StorageError::Decode(s) if s.contains("truncated frame length")),
            "unexpected err: {err}"
        );

        let (loaded, repaired) = load_blockchain_from_disk(&path, &genesis).unwrap();
        assert!(repaired);
        assert_eq!(loaded.height(), 1);
        assert!(BlockStore::read_all_blocks(&path).unwrap().len() == 1);
        let s = loaded.state().get_account(&sender).unwrap();
        assert_eq!(s.nonce, 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_all_blocks_repairing_tail_reports_repaired_without_mid_file_corruption() {
        let path = unique_store_path("repair_flag");
        let _ = std::fs::remove_file(&path);
        let (_, repaired) = BlockStore::read_all_blocks_repairing_tail(&path).unwrap();
        assert!(!repaired);
        let _ = std::fs::remove_file(&path);
    }
}
