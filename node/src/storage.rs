//! Append-only block file: `u32_be frame_len` + `encode_block` payload per record.
//! Genesis is not stored; replay starts from height 1 against [`Blockchain::new`].

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::block::Block;
use crate::blockchain::Blockchain;
use crate::encoding::{decode_block, encode_block, EncodeError};
use crate::errors::ProtocolError;

/// Hard cap per frame to limit hostile allocations when reading untrusted files.
const MAX_FRAME_BYTES: u32 = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum StorageError {
    Io(io::Error),
    Decode(String),
    Replay(ProtocolError),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Io(e) => write!(f, "storage I/O: {e}"),
            StorageError::Decode(s) => write!(f, "storage decode: {s}"),
            StorageError::Replay(e) => write!(f, "chain replay: {e}"),
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
}

impl BlockStore {
    pub fn open_append(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self { file })
    }

    /// Write one frame: `u32_be len` + payload. Syncs to disk for durability.
    pub fn append_payload(&mut self, payload: &[u8]) -> io::Result<()> {
        let len = u32::try_from(payload.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "encoded block exceeds u32 frame length",
            )
        })?;
        self.file.write_all(&len.to_be_bytes())?;
        self.file.write_all(payload)?;
        self.file.sync_all()
    }

    pub fn append_block(&mut self, block: &Block) -> io::Result<()> {
        self.append_payload(&encode_block(block))
    }

    /// Read all blocks in file order (empty file → empty vec).
    pub fn read_all_blocks(path: impl AsRef<Path>) -> Result<Vec<Block>, StorageError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let data = fs::read(path)?;
        let mut out = Vec::new();
        let mut pos = 0usize;
        while pos < data.len() {
            if pos + 4 > data.len() {
                return Err(StorageError::Decode("truncated frame length".into()));
            }
            let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                as usize;
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
}

/// Fresh in-code genesis + replay every stored block through [`Blockchain::append_block`].
///
/// This restores **blocks only**. Anything done out-of-band to [`State`] (for example dev
/// `create_account` funding) is **not** on disk yet; use [`load_blockchain_from_disk_with`] and
/// pass the same bootstrap you used before writing blocks, or add a genesis-state format later.
pub fn load_blockchain_from_disk(path: impl AsRef<Path>) -> Result<Blockchain, StorageError> {
    load_blockchain_from_disk_with(path, |_| {})
}

/// Like [`load_blockchain_from_disk`], but runs `setup` on the empty chain before replay
/// (e.g. recreate initial account balances).
pub fn load_blockchain_from_disk_with(
    path: impl AsRef<Path>,
    setup: impl FnOnce(&mut Blockchain),
) -> Result<Blockchain, StorageError> {
    let blocks = BlockStore::read_all_blocks(path)?;
    let mut chain = Blockchain::new();
    setup(&mut chain);
    for block in blocks {
        chain
            .append_block(block)
            .map_err(StorageError::Replay)?;
    }
    Ok(chain)
}

/// Apply [`Blockchain::append_block`] then persist the same block. If disk fails, the chain
/// is already extended — caller may log and retry `append_block` persistence.
pub fn append_block_and_persist(
    chain: &mut Blockchain,
    store: &mut BlockStore,
    block: Block,
) -> Result<(), StorageError> {
    let payload = encode_block(&block);
    chain.append_block(block).map_err(StorageError::Replay)?;
    store.append_payload(&payload)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
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

    fn signed_tx(seed: u8, receiver: &str, amount: u64, fee: u64, nonce: u64, ts: u64) -> Transaction {
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

    #[test]
    fn append_and_load_roundtrip_with_replay() {
        let path = unique_store_path("roundtrip");
        let _ = std::fs::remove_file(&path);

        let signing_key = SigningKey::from_bytes(&[60u8; 32]);
        let vk = signing_key.verifying_key();
        let sender = Address::new(Crypto::address_from_public_key(&vk.to_bytes()));

        {
            let mut chain = Blockchain::new();
            chain.state_mut().create_account(sender.clone(), 100);
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

        let loaded = load_blockchain_from_disk_with(&path, |c| {
            c.state_mut().create_account(sender.clone(), 100);
        })
        .unwrap();
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
        let chain = load_blockchain_from_disk(&path).unwrap();
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn truncated_file_errors() {
        let path = unique_store_path("trunc");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, &[0u8, 0u8, 0u8, 0x10]).unwrap();
        assert!(load_blockchain_from_disk(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
