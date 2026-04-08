//! Durable peer list with simple failure streaks and cooldown (v2 peer management baseline).

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const DEFAULT_FILE: &str = "peer_book.toml";

/// After this many consecutive failures, peer is skipped until cooldown elapses.
pub const FAIL_STREAK_COOLDOWN_THRESHOLD: u32 = 5;
/// Base cooldown seconds; multiplied by min(fail_streak, 8) for backoff.
pub const COOLDOWN_BASE_SECS: u64 = 30;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerBookFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub peers: Vec<PeerEntry>,
}

fn default_version() -> u32 {
    1
}

fn default_include_in_gossip() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerEntry {
    pub addr: String,
    #[serde(default)]
    pub fail_streak: u32,
    #[serde(default)]
    pub last_fail_unix: u64,
    #[serde(default)]
    pub last_ok_unix: u64,
    /// If false, address is kept for health / UX only (e.g. inbound TCP source) and is omitted from
    /// `OP_PEERS` and from automatic sync candidate lists to avoid gossiping ephemeral client ports.
    #[serde(default = "default_include_in_gossip")]
    pub include_in_gossip: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PeerBook {
    by_addr: HashMap<String, PeerEntry>,
}

impl PeerBook {
    pub fn path_in_data_dir(data_dir: &Path) -> std::path::PathBuf {
        data_dir.join(DEFAULT_FILE)
    }

    pub fn load_or_empty(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file: PeerBookFile =
            toml::from_str(&raw).map_err(|e| format!("{}: peer_book TOML: {e}", path.display()))?;
        let mut b = PeerBook::default();
        for e in file.peers {
            b.by_addr.insert(e.addr.clone(), e);
        }
        Ok(b)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let peers: Vec<PeerEntry> = self.by_addr.values().cloned().collect();
        let file = PeerBookFile { version: 1, peers };
        let body = toml::to_string_pretty(&file).map_err(|e| e.to_string())?;
        let out = format!(
            "# Trilogicon peer book — known peers and connection health (v2).\n\
             # Edit with care; invalid addresses are skipped at runtime.\n\n{body}"
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(path, out).map_err(|e| e.to_string())
    }

    pub fn merge_seed(&mut self, addr: String) {
        if addr.is_empty() {
            return;
        }
        match self.by_addr.get_mut(&addr) {
            Some(e) => {
                e.include_in_gossip = true;
            }
            None => {
                self.by_addr.insert(
                    addr.clone(),
                    PeerEntry {
                        addr,
                        fail_streak: 0,
                        last_fail_unix: 0,
                        last_ok_unix: 0,
                        include_in_gossip: true,
                    },
                );
            }
        }
    }

    /// Like repeated [`Self::merge_seed`], but returns how many addresses were **new** to the book
    /// (existing keys only get `include_in_gossip` refreshed).
    pub fn merge_seeds_report_new(&mut self, addrs: impl IntoIterator<Item = String>) -> usize {
        use std::collections::hash_map::Entry;

        let mut n = 0usize;
        for a in addrs {
            if a.is_empty() {
                continue;
            }
            match self.by_addr.entry(a.clone()) {
                Entry::Occupied(mut o) => {
                    o.get_mut().include_in_gossip = true;
                }
                Entry::Vacant(v) => {
                    v.insert(PeerEntry {
                        addr: a,
                        fail_streak: 0,
                        last_fail_unix: 0,
                        last_ok_unix: 0,
                        include_in_gossip: true,
                    });
                    n += 1;
                }
            }
        }
        n
    }

    /// Learn the remote socket after an inbound v2 HELLO. Not advertised in [`OP_PEERS`].
    pub fn merge_inbound_peer(&mut self, addr: String) {
        if addr.is_empty() {
            return;
        }
        use std::collections::hash_map::Entry;
        match self.by_addr.entry(addr.clone()) {
            Entry::Occupied(_) => {}
            Entry::Vacant(v) => {
                v.insert(PeerEntry {
                    addr,
                    fail_streak: 0,
                    last_fail_unix: 0,
                    last_ok_unix: 0,
                    include_in_gossip: false,
                });
            }
        }
    }

    /// Ordered list: `seeds` first (dedup), then remaining book addresses.
    pub fn sync_candidates(&self, seeds: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::<&str>::new();
        for s in seeds {
            let t = s.trim();
            if t.is_empty() || !seen.insert(t) {
                continue;
            }
            out.push(t.to_string());
        }
        let mut rest: Vec<String> = self
            .by_addr
            .iter()
            .filter(|(k, e)| !seen.contains(k.as_str()) && e.include_in_gossip)
            .map(|(k, _)| k.clone())
            .collect();
        rest.sort();
        out.extend(rest);
        out
    }

    pub fn is_available(&self, addr: &str, now: u64) -> bool {
        let Some(e) = self.by_addr.get(addr) else {
            return true;
        };
        if e.fail_streak < FAIL_STREAK_COOLDOWN_THRESHOLD {
            return true;
        }
        let mult = e.fail_streak.min(8);
        let need = COOLDOWN_BASE_SECS.saturating_mul(mult as u64);
        now.saturating_sub(e.last_fail_unix) >= need
    }

    pub fn filter_available(&self, addrs: &[String], now: u64) -> Vec<String> {
        addrs
            .iter()
            .filter(|a| self.is_available(a, now))
            .cloned()
            .collect()
    }

    /// Sorted unique addresses for wire gossip (capped).
    pub fn gossip_addresses(&self, max: usize) -> Vec<String> {
        let mut rest: Vec<String> = self
            .by_addr
            .iter()
            .filter(|(_, e)| e.include_in_gossip)
            .map(|(k, _)| k.clone())
            .collect();
        rest.sort();
        rest.truncate(max);
        rest
    }

    pub fn record_ok(&mut self, addr: &str) {
        let now = now_unix();
        let e = self.by_addr.entry(addr.to_string()).or_insert(PeerEntry {
            addr: addr.to_string(),
            fail_streak: 0,
            last_fail_unix: 0,
            last_ok_unix: 0,
            include_in_gossip: true,
        });
        e.fail_streak = 0;
        e.last_ok_unix = now;
    }

    pub fn record_fail(&mut self, addr: &str) {
        let now = now_unix();
        let e = self.by_addr.entry(addr.to_string()).or_insert(PeerEntry {
            addr: addr.to_string(),
            fail_streak: 0,
            last_fail_unix: 0,
            last_ok_unix: 0,
            include_in_gossip: true,
        });
        e.fail_streak = e.fail_streak.saturating_add(1);
        e.last_fail_unix = now;
    }

    #[cfg(test)]
    fn last_fail_unix(&self, addr: &str) -> Option<u64> {
        self.by_addr.get(addr).map(|e| e.last_fail_unix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_seeds_report_new_counts_insertions_only() {
        let mut b = PeerBook::default();
        assert_eq!(
            b.merge_seeds_report_new(vec!["127.0.0.1:1".into(), "127.0.0.1:2".into()]),
            2
        );
        assert_eq!(b.merge_seeds_report_new(vec!["127.0.0.1:1".into()]), 0);
        assert_eq!(b.by_addr.len(), 2);
    }

    #[test]
    fn inbound_peer_is_not_gossiped() {
        let mut b = PeerBook::default();
        b.merge_seed("127.0.0.1:4000".into());
        b.merge_inbound_peer("127.0.0.1:5000".into());
        assert_eq!(b.gossip_addresses(10), vec!["127.0.0.1:4000".to_string()]);
        let cand = b.sync_candidates(&[]);
        assert_eq!(cand, vec!["127.0.0.1:4000".to_string()]);
    }

    #[test]
    fn cooldown_blocks_high_streak() {
        let mut b = PeerBook::default();
        b.merge_seed("127.0.0.1:1".into());
        for _ in 0..FAIL_STREAK_COOLDOWN_THRESHOLD {
            b.record_fail("127.0.0.1:1");
        }
        let last = b.last_fail_unix("127.0.0.1:1").unwrap();
        assert!(!b.is_available("127.0.0.1:1", last));
        let need = COOLDOWN_BASE_SECS * (FAIL_STREAK_COOLDOWN_THRESHOLD as u64).min(8);
        assert!(b.is_available("127.0.0.1:1", last + need));
    }
}
