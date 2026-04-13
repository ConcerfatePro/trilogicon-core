//! V2 data-directory binding: ties a directory to a genesis `state_commitment_hex`.
//! Mismatch after bind exists is a hard startup error (`docs/design_notes/v2_persistence_restart.md`).

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::file_lock::ExclusiveFileLock;
use crate::genesis::Genesis;
use crate::operator_msg::PFX_STARTUP;

/// Written under the data directory when genesis is first established or verified.
pub const GENESIS_BIND_FILE: &str = "genesis_bind.toml";

/// Serializes bind creation / verify so two first-run processes cannot clobber each other.
pub const GENESIS_BIND_LOCK_FILE: &str = ".genesis_bind.lock";

/// Exclusive lock held for the lifetime of `node run` so two `run` processes cannot share one data dir.
pub const NODE_RUN_LOCK_FILE: &str = ".node.run.lock";

#[derive(Debug, Deserialize, Serialize)]
struct BindFile {
    state_commitment_hex: String,
}

fn binding_text(hex: &str) -> Result<String, String> {
    let body = BindFile {
        state_commitment_hex: hex.to_string(),
    };
    Ok(format!(
        "# Trilogicon V2 — binds this data directory to a genesis state commitment.\n# Do not edit by hand unless you know the effect on chain.blocks and pending_tx.tril.\n{}\n",
        toml::to_string(&body).map_err(|e| e.to_string())?
    ))
}

fn read_binding_and_verify(path: &Path, expected: &str) -> Result<(), String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("{PFX_STARTUP} {}: {e}", path.display()))?;
    let parsed: BindFile = toml::from_str(&raw).map_err(|e| {
        format!(
            "{PFX_STARTUP} fail-closed: {} invalid genesis bind TOML ({e}); fix or remove file if you intend a new genesis",
            path.display()
        )
    })?;
    if parsed.state_commitment_hex != expected {
        return Err(format!(
            "{PFX_STARTUP} fail-closed: {} genesis state commitment mismatch — directory was bound to `{}`, loaded genesis.toml commits to `{}`. Refuse startup: use matching genesis, a fresh data directory, or (only with care) reset chain per docs/design_notes/v2_persistence_restart.md.",
            path.display(),
            parsed.state_commitment_hex,
            expected
        ));
    }
    Ok(())
}

/// If `genesis_bind.toml` exists, it must match `genesis`. Does **not** read the chain or create the file.
/// Call **before** `load_blockchain_from_disk` so a bad bind fails without touching `chain.blocks`.
pub fn verify_binding_if_present(data_dir: &Path, genesis: &Genesis) -> Result<(), String> {
    let path = data_dir.join(GENESIS_BIND_FILE);
    if !path.exists() {
        return Ok(());
    }
    let expected = genesis.state_commitment_hex().map_err(|e| e.to_string())?;
    read_binding_and_verify(&path, &expected)
}

/// Creates `genesis_bind.toml` when absent (serialized with [`GENESIS_BIND_LOCK_FILE`]).
/// Call only after genesis + `chain.blocks` load has succeeded so failed startup does not install a bind.
pub fn ensure_binding_if_missing(data_dir: &Path, genesis: &Genesis) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;

    let lock_path = data_dir.join(GENESIS_BIND_LOCK_FILE);
    let _bind_lock = ExclusiveFileLock::acquire_exclusive(&lock_path).map_err(|e| {
        format!(
            "{PFX_STARTUP} fail-closed: could not lock {} — {e}",
            lock_path.display()
        )
    })?;

    let path = data_dir.join(GENESIS_BIND_FILE);
    let expected = genesis.state_commitment_hex().map_err(|e| e.to_string())?;

    if path.exists() {
        return read_binding_and_verify(&path, &expected);
    }

    let text = binding_text(&expected)?;
    let tmp = data_dir.join(format!(
        ".genesis_bind.pending.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    let mut f = match OpenOptions::new().create_new(true).write(true).open(&tmp) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            return if path.exists() {
                read_binding_and_verify(&path, &expected)
            } else {
                Err(format!("{}: {e}", tmp.display()))
            };
        }
        Err(e) => return Err(format!("{}: {e}", tmp.display())),
    };

    if let Err(e) = f
        .write_all(text.as_bytes())
        .and_then(|_| f.sync_all())
        .map_err(|e| format!("{}: {e}", tmp.display()))
    {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    drop(f);

    match fs::hard_link(&tmp, &path) {
        Ok(()) => {
            let _ = fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&tmp);
            read_binding_and_verify(&path, &expected)
        }
        Err(e) => {
            if !path.exists() {
                match fs::rename(&tmp, &path) {
                    Ok(()) => Ok(()),
                    Err(e2) => {
                        let _ = fs::remove_file(&tmp);
                        Err(format!(
                            "{}: hard_link failed ({e}); rename failed ({e2})",
                            path.display()
                        ))
                    }
                }
            } else {
                let _ = fs::remove_file(&tmp);
                read_binding_and_verify(&path, &expected)
            }
        }
    }
}

/// Verify an existing bind, then install one if missing. Prefer splitting
/// [`verify_binding_if_present`] / [`ensure_binding_if_missing`] in `run`/`send` so bind creation
/// happens only after `chain.blocks` loads.
pub fn verify_or_create_binding(data_dir: &Path, genesis: &Genesis) -> Result<(), String> {
    verify_binding_if_present(data_dir, genesis)?;
    ensure_binding_if_missing(data_dir, genesis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genesis::{Genesis, GenesisAllocation};
    use crate::types::Address;

    fn sample_genesis(addr: &str) -> Genesis {
        Genesis {
            allocations: vec![GenesisAllocation {
                address: addr.to_string(),
                balance: 100,
            }],
        }
    }

    #[test]
    fn verify_binding_if_present_does_not_create_file() {
        let dir = std::env::temp_dir().join(format!(
            "trilogicon_bind_verify_only_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let g = sample_genesis(&Address::new("only_verify").0);
        verify_binding_if_present(&dir, &g).unwrap();
        assert!(!dir.join(GENESIS_BIND_FILE).exists());
        ensure_binding_if_missing(&dir, &g).unwrap();
        assert!(dir.join(GENESIS_BIND_FILE).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn creates_bind_when_missing() {
        let dir = std::env::temp_dir().join(format!(
            "trilogicon_bind_create_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let g = sample_genesis(&Address::new("alice_bind").0);
        verify_or_create_binding(&dir, &g).unwrap();
        let p = dir.join(GENESIS_BIND_FILE);
        assert!(p.exists());
        verify_or_create_binding(&dir, &g).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_first_bind_all_verify_same_genesis() {
        use std::sync::Arc;
        use std::thread;

        let dir = std::env::temp_dir().join(format!(
            "trilogicon_bind_race_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let g = sample_genesis("alice_race");
        let dir = Arc::new(dir);
        let g = Arc::new(g);
        let mut handles = Vec::new();
        for _ in 0..8 {
            let d = dir.clone();
            let genesis_arc = g.clone();
            handles.push(thread::spawn(move || {
                verify_or_create_binding(d.as_ref(), genesis_arc.as_ref())
            }));
        }
        for h in handles {
            h.join().unwrap().unwrap();
        }
        verify_or_create_binding(dir.as_ref(), g.as_ref()).unwrap();
        let _ = fs::remove_dir_all(&dir.as_ref());
    }

    #[test]
    fn concurrent_first_bind_different_genesis_exactly_one_succeeds() {
        use std::sync::Arc;
        use std::thread;

        let dir = std::env::temp_dir().join(format!(
            "trilogicon_bind_diff_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let g1 = sample_genesis("alice_diff");
        let g2 = sample_genesis("bob_diff");
        assert_ne!(
            g1.state_commitment_hex().unwrap(),
            g2.state_commitment_hex().unwrap()
        );
        let dir = Arc::new(dir);
        let g1 = Arc::new(g1);
        let g2 = Arc::new(g2);
        let h1 = {
            let d = dir.clone();
            let g = g1.clone();
            thread::spawn(move || verify_or_create_binding(d.as_ref(), g.as_ref()))
        };
        let h2 = {
            let d = dir.clone();
            let g = g2.clone();
            thread::spawn(move || verify_or_create_binding(d.as_ref(), g.as_ref()))
        };
        let r1 = h1.join().unwrap();
        let r2 = h2.join().unwrap();
        assert!(
            r1.is_ok() ^ r2.is_ok(),
            "expected one Ok and one Err, got {:?} {:?}",
            r1,
            r2
        );
        let winner = if r1.is_ok() { g1.as_ref() } else { g2.as_ref() };
        verify_or_create_binding(dir.as_ref(), winner).unwrap();
        assert!(
            verify_or_create_binding(dir.as_ref(), if r1.is_ok() {
                g2.as_ref()
            } else {
                g1.as_ref()
            })
            .is_err()
        );
        let _ = fs::remove_dir_all(&dir.as_ref());
    }

    #[test]
    fn mismatch_refuses() {
        let dir = std::env::temp_dir().join(format!(
            "trilogicon_bind_mismatch_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let g1 = sample_genesis("alice_mis");
        verify_or_create_binding(&dir, &g1).unwrap();
        let g2 = sample_genesis("bob_mis");
        assert!(verify_or_create_binding(&dir, &g2).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
