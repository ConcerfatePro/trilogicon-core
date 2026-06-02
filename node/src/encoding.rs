//! V1 canonical binary encoding for blocks and transactions (disk + future wire).
//!
//! Integers are big-endian. Strings and byte blobs are `u32` length-prefixed (UTF-8 for strings).
//! Each encoded block and transaction starts with a 4-byte magic and a 1-byte format version.

use crate::block::Block;
use crate::transaction::Transaction;
use crate::types::Address;

const BLOCK_MAGIC: &[u8; 4] = b"TRB1";
const TX_MAGIC: &[u8; 4] = b"TRT1";
const BLOCK_FORMAT_VERSION: u8 = 1;
const TX_FORMAT_VERSION: u8 = 1;

#[derive(Debug)]
pub struct EncodeError(pub String);

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for EncodeError {}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn take_exact(&mut self, n: usize) -> Result<&'a [u8], EncodeError> {
        if self.remaining() < n {
            return Err(EncodeError(format!(
                "unexpected end of input (need {n} bytes, have {})",
                self.remaining()
            )));
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn take_u8(&mut self) -> Result<u8, EncodeError> {
        Ok(self.take_exact(1)?[0])
    }

    fn take_u32_be(&mut self) -> Result<u32, EncodeError> {
        let b = self.take_exact(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn take_u64_be(&mut self) -> Result<u64, EncodeError> {
        let b = self.take_exact(8)?;
        Ok(u64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn take_bytes(&mut self) -> Result<Vec<u8>, EncodeError> {
        let n = self.take_u32_be()? as usize;
        Ok(self.take_exact(n)?.to_vec())
    }

    fn take_string(&mut self) -> Result<String, EncodeError> {
        let bytes = self.take_bytes()?;
        String::from_utf8(bytes).map_err(|e| EncodeError(format!("invalid utf-8: {e}")))
    }

    fn finish(self) -> Result<(), EncodeError> {
        if self.pos != self.data.len() {
            return Err(EncodeError(format!(
                "trailing bytes after decode: {} bytes",
                self.data.len() - self.pos
            )));
        }
        Ok(())
    }
}

fn push_u32_be(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_u64_be(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_bytes(buf: &mut Vec<u8>, b: &[u8]) {
    let len = u32::try_from(b.len()).expect("blob length fits u32");
    push_u32_be(buf, len);
    buf.extend_from_slice(b);
}

fn push_string(buf: &mut Vec<u8>, s: &str) {
    push_bytes(buf, s.as_bytes());
}

/// Encodes a transaction (nested inside blocks or standalone).
pub fn encode_transaction(tx: &Transaction) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(TX_MAGIC);
    out.push(TX_FORMAT_VERSION);
    push_string(&mut out, &tx.sender.0);
    push_string(&mut out, &tx.receiver.0);
    push_u64_be(&mut out, tx.amount);
    push_u64_be(&mut out, tx.fee);
    push_u64_be(&mut out, tx.nonce);
    push_u64_be(&mut out, tx.timestamp_unix);
    push_bytes(&mut out, &tx.public_key);
    push_bytes(&mut out, &tx.signature);
    push_string(&mut out, &tx.tx_hash);
    out
}

pub fn decode_transaction(data: &[u8]) -> Result<Transaction, EncodeError> {
    let mut c = Cursor::new(data);
    if c.take_exact(4)? != TX_MAGIC {
        return Err(EncodeError("bad transaction magic".into()));
    }
    let ver = c.take_u8()?;
    if ver != TX_FORMAT_VERSION {
        return Err(EncodeError(format!("unsupported tx format version {ver}")));
    }
    let tx = Transaction {
        sender: Address::new(c.take_string()?),
        receiver: Address::new(c.take_string()?),
        amount: c.take_u64_be()?,
        fee: c.take_u64_be()?,
        nonce: c.take_u64_be()?,
        timestamp_unix: c.take_u64_be()?,
        public_key: c.take_bytes()?,
        signature: c.take_bytes()?,
        tx_hash: c.take_string()?,
    };
    c.finish()?;
    Ok(tx)
}

/// Encodes a full block (non-genesis blocks are what we persist; genesis is in-code only).
pub fn encode_block(block: &Block) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(BLOCK_MAGIC);
    out.push(BLOCK_FORMAT_VERSION);
    push_u64_be(&mut out, block.height);
    push_string(&mut out, &block.previous_hash);
    push_u64_be(&mut out, block.timestamp_unix);
    let n = u32::try_from(block.transactions.len()).expect("tx count fits u32");
    push_u32_be(&mut out, n);
    for tx in &block.transactions {
        let nested = encode_transaction(tx);
        let len = u32::try_from(nested.len()).expect("tx encoding length fits u32");
        push_u32_be(&mut out, len);
        out.extend_from_slice(&nested);
    }
    push_string(&mut out, &block.block_hash);
    out
}

pub fn decode_block(data: &[u8]) -> Result<Block, EncodeError> {
    let mut c = Cursor::new(data);
    if c.take_exact(4)? != BLOCK_MAGIC {
        return Err(EncodeError("bad block magic".into()));
    }
    let ver = c.take_u8()?;
    if ver != BLOCK_FORMAT_VERSION {
        return Err(EncodeError(format!(
            "unsupported block format version {ver}"
        )));
    }
    let height = c.take_u64_be()?;
    let previous_hash = c.take_string()?;
    let timestamp_unix = c.take_u64_be()?;
    let n = c.take_u32_be()? as usize;
    let mut transactions = Vec::with_capacity(n);
    for i in 0..n {
        let len = c.take_u32_be()? as usize;
        let chunk = c.take_exact(len)?;
        let tx = decode_transaction(chunk).map_err(|e| EncodeError(format!("tx {i}: {e}")))?;
        transactions.push(tx);
    }
    let block_hash = c.take_string()?;
    c.finish()?;
    Ok(Block {
        height,
        previous_hash,
        timestamp_unix,
        transactions,
        block_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
    use ed25519_dalek::{Signer, SigningKey};

    fn sample_tx() -> Transaction {
        let signing_key = SigningKey::from_bytes(&[3u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let mut tx = Transaction {
            sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
            receiver: Address::new("recv_enc"),
            amount: 7,
            fee: 2,
            nonce: 1,
            timestamp_unix: 1_700_100_000,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let payload = tx.unsigned_payload_bytes();
        tx.signature = signing_key.sign(&payload).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&payload);
        tx
    }

    #[test]
    fn transaction_roundtrip() {
        let tx = sample_tx();
        let bytes = encode_transaction(&tx);
        let back = decode_transaction(&bytes).unwrap();
        assert_eq!(tx.sender, back.sender);
        assert_eq!(tx.receiver, back.receiver);
        assert_eq!(tx.amount, back.amount);
        assert_eq!(tx.fee, back.fee);
        assert_eq!(tx.nonce, back.nonce);
        assert_eq!(tx.timestamp_unix, back.timestamp_unix);
        assert_eq!(tx.public_key, back.public_key);
        assert_eq!(tx.signature, back.signature);
        assert_eq!(tx.tx_hash, back.tx_hash);
    }

    #[test]
    fn block_roundtrip() {
        let mut block = Block {
            height: 3,
            previous_hash: "parent_hash_hex".into(),
            timestamp_unix: 1_700_200_000,
            transactions: vec![sample_tx()],
            block_hash: String::new(),
        };
        block.block_hash = block.compute_block_hash();
        let bytes = encode_block(&block);
        let back = decode_block(&bytes).unwrap();
        assert_eq!(block.height, back.height);
        assert_eq!(block.previous_hash, back.previous_hash);
        assert_eq!(block.timestamp_unix, back.timestamp_unix);
        assert_eq!(block.transactions.len(), back.transactions.len());
        assert_eq!(block.block_hash, back.block_hash);
        assert!(back.basic_validate().is_ok());
    }

    #[test]
    fn decode_transaction_rejects_bad_magic() {
        let mut bad = encode_transaction(&sample_tx());
        bad[0] ^= 0xFF;
        assert!(decode_transaction(&bad).is_err());
    }

    #[test]
    fn decode_transaction_rejects_truncated_input() {
        assert!(decode_transaction(&[]).is_err());
        assert!(decode_transaction(b"TRT").is_err());
    }

    #[test]
    fn decode_transaction_rejects_trailing_bytes() {
        let mut bytes = encode_transaction(&sample_tx());
        bytes.push(0);
        assert!(decode_transaction(&bytes).is_err());
    }

    #[test]
    fn decode_block_rejects_bad_magic() {
        let mut block = Block {
            height: 1,
            previous_hash: "p".into(),
            timestamp_unix: 1,
            transactions: vec![sample_tx()],
            block_hash: String::new(),
        };
        block.block_hash = block.compute_block_hash();
        let mut bytes = encode_block(&block);
        bytes[1] ^= 0xFF;
        assert!(decode_block(&bytes).is_err());
    }

    /// Smoke property: random bytes must never panic `decode_*` (only `Result`).
    #[test]
    fn decode_random_inputs_do_not_panic() {
        use rand::RngCore;

        let mut rng = rand::rng();
        let mut buf = [0u8; 384];
        for _ in 0..2500 {
            rng.fill_bytes(&mut buf);
            let _ = decode_transaction(&buf);
            let n = usize::from(buf[1]) % buf.len() + 1;
            let _ = decode_block(&buf[..n]);
        }
    }
}
