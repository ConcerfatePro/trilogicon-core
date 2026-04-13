//! Safe `pending_tx.tril` drain: parse is all-or-nothing; file is only cleared or rewritten after
//! mempool acceptance is accounted for (`docs/design_notes/v2_persistence_restart.md`).
//!
//! All access to the queue file is serialized with [`PENDING_TX_LOCK_FILE`] so `send` (append) and
//! `run` (drain) cannot race each other into silent loss or torn frames.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::encoding::{decode_transaction, encode_transaction};
use crate::errors::ProtocolError;
use crate::file_lock::ExclusiveFileLock;
use crate::mempool::Mempool;
use crate::operator_msg::PFX_PENDING;
use crate::transaction::Transaction;

/// Max encoded tx frame on disk (matches CLI `send` path).
pub const MAX_PENDING_TX_FRAME: u32 = 4 * 1024 * 1024;

/// Advisory lock file next to `pending_tx.tril` (same directory). Held for full append and drain.
pub const PENDING_TX_LOCK_FILE: &str = ".pending_tx.lock";

/// Path to the lock file for a given `pending_tx.tril` path.
pub fn pending_queue_lock_path(pending_tx_path: &Path) -> PathBuf {
    pending_tx_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join(PENDING_TX_LOCK_FILE)
}

/// Parse every frame without modifying the file. Returns `Err` on any truncation or decode error.
pub fn parse_pending_file_bytes(data: &[u8]) -> Result<Vec<Transaction>, String> {
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
    Ok(out)
}

fn encode_pending_payload(txs: &[Transaction]) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    for tx in txs {
        let payload = encode_transaction(tx);
        let len = u32::try_from(payload.len()).map_err(|_| {
            "encoded transaction length exceeds u32".to_string()
        })?;
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&payload);
    }
    Ok(buf)
}

fn write_pending_atomic(path: &Path, payload: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tril.tmp");
    fs::write(&tmp, payload)?;
    {
        let f = File::open(&tmp)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            match File::open(dir) {
                Ok(d) => {
                    if let Err(e) = d.sync_all() {
                        eprintln!(
                            "{PFX_PENDING} warning: pending_tx.tril rename ok but directory fsync failed ({e}) — see v2_persistence_restart.md"
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "{PFX_PENDING} warning: could not open data dir for fsync after pending rewrite ({e})"
                    );
                }
            }
        }
    }
    Ok(())
}

/// Append one tx as a single length-prefixed frame write (+ sync). Serialized with
/// [`drain_pending_file`] via [`PENDING_TX_LOCK_FILE`].
pub fn append_pending_transaction(path: impl AsRef<Path>, tx: &Transaction) -> std::io::Result<()> {
    let path = path.as_ref();
    let lock_path = pending_queue_lock_path(path);
    let _lock = ExclusiveFileLock::acquire_exclusive(&lock_path).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "{PFX_PENDING} fail-closed: could not acquire queue lock {} — {e}",
                lock_path.display()
            ),
        )
    })?;

    let payload = encode_transaction(tx);
    let len = u32::try_from(payload.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "encoded transaction length exceeds u32",
        )
    })?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&payload);
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(&frame)?;
    f.sync_all()
}

/// Drain queued txs into `pool` using head-of-line acceptance: stop at the first mempool reject
/// (except duplicates, which consume the queue entry). Duplicate `tx_hash` is treated as success
/// for durability (already represented in the pool).
///
/// On parse error, the file is **not** modified. On success, the file is replaced with the encoded
/// **remaining** tail (possibly empty).
pub fn drain_pending_file(path: &Path, pool: &mut Mempool) -> Result<(), String> {
    let lock_path = pending_queue_lock_path(path);
    let _lock = ExclusiveFileLock::acquire_exclusive(&lock_path).map_err(|e| {
        format!(
            "{PFX_PENDING} fail-closed: could not acquire queue lock {} — {e}",
            lock_path.display()
        )
    })?;

    if !path.exists() {
        return Ok(());
    }
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let txs = parse_pending_file_bytes(&data)?;
    let pool_before = pool.clone_fifo();

    if txs.is_empty() {
        if let Err(e) = write_pending_atomic(path, &[]) {
            pool.restore_fifo(pool_before);
            return Err(e.to_string());
        }
        return Ok(());
    }

    let mut i = 0usize;
    while i < txs.len() {
        let tx = txs[i].clone();
        match pool.try_submit_pending_drain(tx) {
            Ok(()) => i += 1,
            Err(ProtocolError::DuplicateTransaction) => i += 1,
            Err(_) => break,
        }
    }

    let remaining = &txs[i..];
    let encoded = encode_pending_payload(remaining)?;
    if let Err(e) = write_pending_atomic(path, &encoded) {
        pool.restore_fifo(pool_before);
        return Err(e.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
    use crate::types::Address;
    use ed25519_dalek::{Signer, SigningKey};

    fn signed_tx(seed: u8, receiver: &str, nonce: u64) -> Transaction {
        let signing_key = SigningKey::from_bytes(&[seed; 32]);
        let verifying_key = signing_key.verifying_key();
        let mut tx = Transaction {
            sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
            receiver: Address::new(receiver),
            amount: 1,
            fee: 1,
            nonce,
            timestamp_unix: 1_701_000_000 + nonce,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let p = tx.unsigned_payload_bytes();
        tx.signature = signing_key.sign(&p).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&p);
        tx
    }

    fn append_frame(buf: &mut Vec<u8>, tx: &Transaction) {
        let payload = encode_transaction(tx);
        let len = u32::try_from(payload.len()).unwrap();
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&payload);
    }

    #[test]
    fn parse_error_leaves_file_bytes_unchanged() {
        let dir = std::env::temp_dir().join(format!(
            "trilogicon_pend_parse_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pending_tx.tril");
        fs::write(&path, [0u8, 0u8, 0u8, 0x10]).unwrap();
        let before = fs::read(&path).unwrap();
        let mut pool = Mempool::new(100);
        assert!(drain_pending_file(&path, &mut pool).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_accepted_clears_file() {
        let dir = std::env::temp_dir().join(format!(
            "trilogicon_pend_all_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pending_tx.tril");
        let t = signed_tx(77, "r1", 0);
        let mut buf = Vec::new();
        append_frame(&mut buf, &t);
        fs::write(&path, &buf).unwrap();

        let mut pool = Mempool::new(100);
        drain_pending_file(&path, &mut pool).unwrap();
        assert!(fs::read(&path).unwrap().is_empty());
        assert_eq!(pool.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn head_reject_preserves_tail_in_order() {
        let dir = std::env::temp_dir().join(format!(
            "trilogicon_pend_hol_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pending_tx.tril");
        let first = signed_tx(78, "r2", 0);
        let second = signed_tx(79, "r3", 0);
        let mut buf = Vec::new();
        append_frame(&mut buf, &first);
        append_frame(&mut buf, &second);
        fs::write(&path, &buf).unwrap();

        // Capacity 1: first pending tx cannot enter the pool.
        let mut pool = Mempool::new(1);
        let filler = signed_tx(80, "filler", 0);
        pool.try_submit(filler).unwrap();

        drain_pending_file(&path, &mut pool).unwrap();
        assert_eq!(pool.len(), 1);
        let rest = fs::read(&path).unwrap();
        let parsed = parse_pending_file_bytes(&rest).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].tx_hash, first.tx_hash);
        assert_eq!(parsed[1].tx_hash, second.tx_hash);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_pending_transaction_writes_one_complete_frame() {
        let dir = std::env::temp_dir().join(format!(
            "trilogicon_pend_append1_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pending_tx.tril");
        let t = signed_tx(81, "r_append", 0);
        append_pending_transaction(&path, &t).unwrap();
        let raw = fs::read(&path).unwrap();
        let parsed = parse_pending_file_bytes(&raw).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].tx_hash, t.tx_hash);
        assert_eq!(raw.len(), 4 + encode_transaction(&t).len());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn drain_pending_rewrite_failure_restores_mempool() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "trilogicon_pend_rwfail_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pending_tx.tril");
        let t = signed_tx(82, "r_rwfail", 0);
        let mut buf = Vec::new();
        append_frame(&mut buf, &t);
        fs::write(&path, &buf).unwrap();

        let mut pool = Mempool::new(100);
        let before_hashes: Vec<_> = pool.clone_fifo().iter().map(|x| x.tx_hash.clone()).collect();

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        let err = drain_pending_file(&path, &mut pool).expect_err("drain should fail closed");
        assert!(
            err.contains("fail-closed") || err.contains("Permission"),
            "unexpected err (lock or rewrite): {err}"
        );

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            pool.clone_fifo()
                .iter()
                .map(|x| x.tx_hash.clone())
                .collect::<Vec<_>>(),
            before_hashes
        );
        let file_after = fs::read(&path).unwrap();
        assert_eq!(file_after, buf, "pending file must be unchanged on failure");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_waits_until_queue_lock_released() {
        use std::thread;
        use std::time::{Duration, Instant};

        let dir = std::env::temp_dir().join(format!(
            "trilogicon_pend_lockwait_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pending_tx.tril");
        let lock_path = pending_queue_lock_path(&path);

        let holder = ExclusiveFileLock::acquire_exclusive(&lock_path).unwrap();
        let t = signed_tx(90, "r_lockwait", 0);
        let want_hash = t.tx_hash.clone();
        let path_c = path.clone();
        let h = thread::spawn(move || append_pending_transaction(&path_c, &t));

        let start = Instant::now();
        let mut saw_blocked = false;
        while start.elapsed() < Duration::from_secs(2) {
            if !h.is_finished() {
                saw_blocked = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            saw_blocked,
            "append should block while another holder keeps the queue lock"
        );

        drop(holder);
        h.join().unwrap().unwrap();
        let parsed = parse_pending_file_bytes(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].tx_hash, want_hash);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_appends_produce_only_complete_frames() {
        use std::sync::Arc;
        use std::thread;

        let dir = std::env::temp_dir().join(format!(
            "trilogicon_pend_conc_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = Arc::new(dir.join("pending_tx.tril"));
        const N: usize = 6;
        const M: usize = 20;
        let mut handles = Vec::new();
        for tid in 0..N {
            let p = path.clone();
            handles.push(thread::spawn(move || {
                for j in 0..M {
                    let seed = (tid * M + j) as u8;
                    let tx = signed_tx(seed, &format!("recv_{tid}_{j}"), j as u64);
                    append_pending_transaction(&*p, &tx).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let raw = fs::read(path.as_ref()).unwrap();
        let parsed = parse_pending_file_bytes(&raw).unwrap();
        assert_eq!(parsed.len(), N * M);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn interleaved_send_and_drain_no_silent_loss() {
        use std::sync::{Arc, Mutex};
        use std::thread;

        let dir = std::env::temp_dir().join(format!(
            "trilogicon_pend_inter_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = Arc::new(dir.join("pending_tx.tril"));
        let pool_shared = Arc::new(Mutex::new(Mempool::new(500)));

        let p_send = path.clone();
        let h_send = thread::spawn(move || {
            for i in 0..40 {
                let tx = signed_tx((100 + i) as u8, "inter_recv", i);
                append_pending_transaction(&*p_send, &tx).unwrap();
            }
        });

        let p_drain = path.clone();
        let ps = pool_shared.clone();
        let h_drain = thread::spawn(move || {
            for _ in 0..500 {
                let mut pool = ps.lock().expect("pool lock");
                let _ = drain_pending_file(&*p_drain, &mut *pool);
                thread::yield_now();
            }
        });

        h_send.join().unwrap();
        h_drain.join().unwrap();

        let mut pool = pool_shared.lock().expect("pool lock");
        drain_pending_file(&*path, &mut *pool).unwrap();
        assert_eq!(
            pool.len(),
            40,
            "all sent txs must be accounted for in the mempool (shared across drain passes)"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_submission_consumes_pending_entry() {
        let dir = std::env::temp_dir().join(format!(
            "trilogicon_pend_dup_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pending_tx.tril");
        let t = signed_tx(79, "r4", 0);
        let mut buf = Vec::new();
        append_frame(&mut buf, &t);
        append_frame(&mut buf, &t);
        fs::write(&path, &buf).unwrap();

        let mut pool = Mempool::new(100);
        drain_pending_file(&path, &mut pool).unwrap();
        assert_eq!(pool.len(), 1);
        assert!(fs::read(&path).unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
