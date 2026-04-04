//! Append-only block file: `u32_be frame_len` + `encode_block` payload per record.
//! Genesis **block** is not stored; replay applies stored blocks on top of
//! [`Blockchain::from_genesis`] using the same protocol [`Genesis`](crate::genesis::Genesis) document.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::block::Block;
use crate::blockchain::Blockchain;
use crate::encoding::{EncodeError, decode_block, encode_block};
use crate::errors::ProtocolError;
use crate::genesis::Genesis;

/// Hard cap per frame to limit hostile allocations when reading untrusted files.
const MAX_FRAME_BYTES: u32 = 64 * 1024 * 1024;

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
}

impl BlockStore {
    pub fn open_append(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
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

fn chain_from_genesis(genesis: &Genesis) -> Result<Blockchain, StorageError> {
    Blockchain::from_genesis(genesis).map_err(|e| match e {
        ProtocolError::GenesisError(s) => StorageError::Genesis(s),
        other => StorageError::Replay(other),
    })
}

/// Build chain from `genesis` (height-0 state), then apply every stored block in file order.
/// Empty or missing file → chain is genesis tip only.
pub fn load_blockchain_from_disk(
    path: impl AsRef<Path>,
    genesis: &Genesis,
) -> Result<Blockchain, StorageError> {
    let blocks = BlockStore::read_all_blocks(path)?;
    let mut chain = chain_from_genesis(genesis)?;
    for block in blocks {
        chain.append_block(block).map_err(StorageError::Replay)?;
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

        let loaded = load_blockchain_from_disk(&path, &genesis).unwrap();
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
        let chain = load_blockchain_from_disk(&path, &Genesis::empty()).unwrap();
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn truncated_file_errors() {
        let path = unique_store_path("trunc");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, [0u8, 0u8, 0u8, 0x10]).unwrap();
        assert!(load_blockchain_from_disk(&path, &Genesis::empty()).is_err());
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

        match load_blockchain_from_disk(&path, &genesis) {
            Ok(_) => panic!("expected decode error"),
            Err(e) => assert!(
                matches!(&e, StorageError::Decode(s) if s.contains("truncated frame length")),
                "unexpected err: {e}"
            ),
        }
        let _ = std::fs::remove_file(&path);
    }
}
