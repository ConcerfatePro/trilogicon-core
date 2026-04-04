# Trilogicon node modules — ownership

Each row is the **single primary role** of the module. Validity rules belong in core modules, not in the CLI.

| Module | Owns | Does **not** own | Invariants callers rely on |
|--------|------|------------------|----------------------------|
| `types` | Address/account primitives and validation helpers for those types | Transaction or block semantics | Address strings used in state/tx are consistent with `Address` rules |
| `crypto` | Hashing, Ed25519 verify/sign helpers | Payload field ordering for signing | Deterministic hashes for a given byte input |
| `transaction` | Tx structure, canonical unsigned payload, `basic_validate` (sig, hash, binding, amounts) | Account balances, chain position | A tx passing `basic_validate` is self-consistent; state may still reject it |
| `block` | Block structure, header preimage, `basic_validate` (tx set, hashes, genesis rules) | Applying txs to state | Non-genesis blocks list no duplicate `tx_hash`; txs pass `basic_validate` |
| `state` | Account map, `from_genesis`, `apply_transaction` (balance, nonce, fee burn) | Mempool policy, networking | `apply_transaction` is deterministic; rejects wrong nonce / balance atomically |
| `genesis` | Genesis file format, sorted allocations, building initial `State` inputs | Runtime peer configuration | Same genesis file ⇒ same initial state across nodes |
| `consensus` | Timestamp policy vs parent and optional local clock drift | Tx validity, block tx lists | Params are explicit; default is permissive for tests |
| `blockchain` | Chain list, **only** `append_block` / `try_append_network_block` / `append_block_from_mempool` as state mutators | Wire framing, wallet | Tip height/hash link; full-block atomic state apply |
| `mempool` | FIFO queue, `basic_validate` on submit, capacity, dedup by `tx_hash` | Balance/nonce checks at submit time | Submitted txs are structurally valid; state checked at seal |
| `encoding` | Canonical binary layout for tx/block (disk + wire body) | Protocol semantics beyond “decode succeeded” | Roundtrip encode/decode preserves fields |
| `storage` | `chain.blocks` persistence, `load_blockchain_from_disk` replay | Validation beyond calling `append_block` | Replay applies blocks in file order with given genesis |
| `network` | TCP framing, opcodes, `NodeInner::append_network_block_persist` glue | New validity rules | Ingress blocks go through `try_append_network_block` |
| `wallet` | Key material from seed, signing for local `send` | Chain selection or sync | Signatures match `transaction` payload rules |
| `main` (binary) | CLI parsing, process lifecycle, threading peers/listen, **calling** core APIs | Any new validity rule not expressed in core | State changes only via `Blockchain` / `Mempool` / storage helpers |

## Consensus boundary

Anything that decides whether a block or transaction **may change committed state** must be reachable through:

- `Blockchain::append_block` (after any ingress-only pre-checks such as `validate_block_vs_local_time` on the network path), or
- `State::apply_transaction` as invoked from that path.

The CLI must not apply balances or nonces except by these code paths.
