# Trilogicon node modules — ownership

Each row is the **single primary role** of the module. Validity rules belong in core modules, not in the CLI.

| Module | Owns | Does **not** own | Invariants callers rely on |
|--------|------|------------------|----------------------------|
| `types` | Address/account primitives and validation helpers for those types | Transaction or block semantics | Address strings used in state/tx are consistent with `Address` rules |
| `crypto` | Hashing, Ed25519 verify/sign helpers | Payload field ordering for signing | Deterministic hashes for a given byte input |
| `transaction` | Tx structure, canonical unsigned payload, `basic_validate` (sig, hash, binding, amounts) | Account balances, chain position | A tx passing `basic_validate` is self-consistent; state may still reject it |
| `block` | Block structure, header preimage, `basic_validate` (tx set, hashes, genesis rules) | Applying txs to state | Non-genesis blocks list no duplicate `tx_hash`; txs pass `basic_validate` |
| `state` | Account map, `from_genesis`, `apply_transaction` (balance, nonce, fee burn) with **checked** arithmetic on balance/nonce updates | Mempool policy, networking | `apply_transaction` is deterministic; rejects wrong nonce / balance / overflow atomically (no partial mutation) |
| `genesis` | Genesis file format, sorted allocations, building initial `State` inputs | Runtime peer configuration | Same genesis file ⇒ same initial state across nodes |
| `consensus` | Timestamp policy vs parent and optional local clock drift | Tx validity, block tx lists | Params are explicit; default is permissive for tests |
| `blockchain` | Chain list, **only** `append_block` / `try_append_network_block` / `append_block_from_mempool` as state mutators | Wire framing, wallet | Tip height/hash link; full-block atomic state apply |
| `mempool` | FIFO queue, `basic_validate` on submit, capacity (`--mempool-capacity` in `main`), dedup by `tx_hash`; **V2:** at most one queued tx per **(sender, nonce)** (reject later distinct `tx_hash`); `purge_nonviable_under_committed_state` (FIFO head vs committed), `drop_stale_nonces_vs_committed`, `drop_later_sender_nonce_conflicts_keep_fifo_first`, `hygiene_vs_committed_ledger` — also invoked after inbound block persist and after sync catch-up (via `network` / `main`) | Balance/nonce checks at submit time | Submitted txs are structurally valid; state checked at seal; hygiene is local only (no reorder / no skip-within-seal); atomic prefix seal can still stick if a **later** prefix tx is invalid |
| `encoding` | Canonical binary layout for tx/block (disk + wire body) | Protocol semantics beyond “decode succeeded” | Roundtrip encode/decode preserves fields |
| `storage` | `chain.blocks`: **V2** magic + CRC-framed records for new files; **legacy** length-prefixed frames still supported; `load_blockchain_from_disk` replay; fail-closed load; `BlockStore` poison is **in-process** only (see [`v2_persistence_restart.md`](design_notes/v2_persistence_restart.md)) | Validation beyond calling `append_block` | Replay applies blocks in file order with given genesis |
| `data_dir_bind` | `genesis_bind.toml` create/verify vs `genesis.toml` commitment | Chain or mempool rules | Mismatch after bind exists → startup error (`run` / `send`) |
| `pending_tx_file` | Safe `pending_tx.tril` parse + drain into mempool; temp-file fsync before rename; Unix parent-dir fsync after rename (best-effort) | Transaction validity | Head-of-line mempool submit; atomic file rewrite; parse errors do not truncate the file |
| `network` | TCP framing, V2 session handshake (genesis commitment + wire version), bounded `OP_BLOCKS` batches, linear sync loop (`sync_from_peer` runs mempool hygiene after appended blocks), inbound caps / idle + write deadlines / per-session error & frame budgets, per-session ingress quotas (stale decoded `OP_BLOCK`, decoded `OP_TX`), inbound decode/preflight (`predecode_inbound_app_payload`) outside the `NodeInner` lock where safe; `NodeInner::append_network_block_persist` + post-append mempool hygiene; `NetworkBlockPersistFailure` / `PeerFrameError` classify outcomes without substring heuristics; stderr lines tagged `[peer]` / `[sync]` / `[storage]` where applicable | New validity rules | Ingress blocks go through `try_append_network_block`; handshake mismatch → disconnect; peer height in session is advisory only; resource limits are local-only |
| `operator_msg` | Stable `[subsystem]` string constants for operator-facing stderr (`README.md`) | Protocol or validity rules | Prefixes document intent only; no behavior |
| `wallet` | Key material from seed, signing for local `send` | Chain selection or sync | Signatures match `transaction` payload rules |
| `main` (binary) | CLI parsing, process lifecycle, threading peers/listen, **calling** core APIs; **V2:** exclusive `.node.run.lock` for `run` only; optional **sealing gate** when `--peers` is non-empty until all peers succeed catch-up | Any new validity rule not expressed in core | State changes only via `Blockchain` / `Mempool` / storage helpers |

## Consensus boundary

Anything that decides whether a block or transaction **may change committed state** must be reachable through:

- `Blockchain::append_block` (after any ingress-only pre-checks such as `validate_block_vs_local_time` on the network path), or
- `State::apply_transaction` as invoked from that path.

The CLI must not apply balances or nonces except by these code paths.

**V2 planning note:** V2 may add **session metadata**, **storage layout/version stamps**, and **handshake fields** for operability, but **no new block or transaction acceptance rule** may exist **only** as CLI or runtime config—validity must remain defined in core modules and stay aligned with [`docs/v2_scope.md`](v2_scope.md) ([Protocol freeze](v2_scope.md#protocol-freeze-for-v2), [Project decisions (V2)](v2_scope.md#project-decisions-v2)). Existing flags that already affect ingress (e.g. timestamp drift limits) are **consensus-sensitive** and require **deployment-wide standardization** per `docs/v2_scope.md`; changing their semantics is a **protocol version** change, not a config tweak.
