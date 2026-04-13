# Trilogicon V1 — Protocol genesis

## What is consensus-critical

For V1, **honest nodes must agree on two things** before they apply any non-genesis block:

1. **The genesis block** — Fixed in code as [`Block::genesis`](../node/src/block.rs): height `0`, empty transaction list, constant hashes (`GENESIS` / `GENESIS_HASH`). It is **not** stored in `chain.blocks`; it exists only in memory when the node starts.

2. **The genesis state** — Declared by a shared **genesis document** (TOML) listing **initial balance allocations** at height `0`. The [`Genesis`](../node/src/genesis.rs) type loads this file; [`State::from_genesis`](../node/src/state.rs) builds the account map **deterministically** (sorted by address string, no duplicate addresses).

If two operators run with **different** genesis documents, they will disagree on balances and nonces even if they exchange identical blocks. **Compare the `state_commitment_hex` value** (printed when you create a genesis file with `init --genesis-balance`, and embedded as a comment in written TOML) across nodes before joining a network.

## How nodes initialize from empty storage

1. Load the **same** `genesis.toml` (or path passed with `--genesis`).
2. `Blockchain::from_genesis` creates the in-memory genesis block + state from allocations.
3. If `chain.blocks` on disk is **missing or empty**, the canonical chain is **genesis only** (height `0`).
4. If frames exist on disk, each stored block is replayed with [`append_block`](../node/src/blockchain.rs), which enforces linkage, tx validity, and state transitions — **no** `create_account` side channel.

The CLI command `init --genesis-balance N` creates `wallet.seed` and writes `genesis.toml` with **one** allocation for that wallet. For **multiple** participants, merge allocations into a **single** shared genesis file (same commitment on every machine) and copy it to each node’s data directory (or point `--genesis` at a shared path).

## Out of scope for V1 (candidates for later protocol versions)

These items are **not part of V1**. Which release they target is decided in [`v2_scope.md`](v2_scope.md). **Do not treat this list as a V2 backlog**—most entries are **post–V2** protocol work (fork choice, snapshots, governance, chain ID, rich genesis).

### V2 vs later: genesis and identity

**Reasonable V2 hardening (node software, same V1 protocol):** clearer **genesis mismatch detection** (e.g. compare `state_commitment_hex` before trusting sync), **operator-visible errors**, optional **data-dir metadata** recording which genesis a directory was created for, and documentation for recovery when operators mix files. **Local** genesis/binding mismatch is a **startup refusal**, not a warning. See [`v2_scope.md`](v2_scope.md) ([Project decisions (V2)](v2_scope.md#project-decisions-v2), [`design_notes/v2_persistence_restart.md`](design_notes/v2_persistence_restart.md)).

**Post–V2 protocol changes (not V2 node-hardening):** **Chain ID inside signed transactions** and **rich genesis** (validators, consensus params in the genesis document) alter **validity, identity, or economics**; they require a **versioned protocol** release, not V2 alone. Until then, cross-network replay remains an **operator** concern.

**Later-protocol candidates (same list as before; not a V2 backlog):**

- **Fork choice / longest chain** — V1 assumes a **linear** extension; no reorg protocol.
- **State snapshots over the wire** — Followers replay **blocks** only; they do not download a Merkleized state dump. (If someone is not in genesis and never receives a credit in a block, they cannot spend.)
- **On-chain governance of genesis** — Genesis is **off-chain social consensus** among operators.
- **Chain ID inside signed transactions** — **Post–V2** if adopted; see paragraph above.
- **Rich genesis** (validators, extra params) — **Post–V2**; today [`ConsensusParams`](../node/src/consensus.rs) are node-local except where operators standardize socially.

## Related code

| Piece | Role |
|--------|------|
| `node/src/genesis.rs` | TOML format, validation, state commitment |
| `node/src/state.rs` | `from_genesis`, `accounts_sorted` |
| `node/src/blockchain.rs` | `from_genesis`, `append_block` |
| `node/src/storage.rs` | `load_blockchain_from_disk(path, genesis) -> (Blockchain, repaired)` |
| `node/src/main.rs` | `--genesis`, `init --genesis-balance` |
| `node/tests/genesis_convergence.rs` | Two-node convergence without manual funding |
