# Module Map

This is the short map of where behavior lives.

| Area | Files |
|------|-------|
| Transactions | `transaction.rs`, `wallet.rs`, `crypto.rs`, `types.rs` |
| State | `state.rs` |
| Blocks and chain | `block.rs`, `blockchain.rs` |
| Genesis | `genesis.rs`, `data_dir_bind.rs` |
| Storage | `storage.rs`, `pending_tx_file.rs`, `file_lock.rs` |
| Network | `network.rs`, `peer_book.rs`, `seen.rs` |
| CLI | `main.rs`, `diag.rs`, `operator_msg.rs` |
| V3 planning | `v3/*` |

## Consensus boundary

Consensus-sensitive behavior is mostly in `transaction.rs`, `state.rs`, `block.rs`, `blockchain.rs`, and `genesis.rs`.

V2 network, persistence, logging, and mempool work should not change what a valid block or transaction means.

V3 modules are not live entrypoints. Do not call them from `append_block`, network ingest, storage migration, or CLI unless the V3 gate is explicitly cleared.
