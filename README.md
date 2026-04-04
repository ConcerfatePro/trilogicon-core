# Trilogicon (TRIL)

Minimal V1 value-transfer chain in Rust: accounts, signed transfers, blocks, deterministic replay, shared **genesis**, and basic TCP sync between nodes.

Design rules live under [`docs/`](docs/) (scope, invariants, architecture). **Genesis** is documented in [`docs/genesis.md`](docs/genesis.md). Module ownership and the consensus boundary are summarized in [`docs/modules.md`](docs/modules.md); the V1 completion checklist is [`docs/v1_checkpoint.md`](docs/v1_checkpoint.md).

## Build

```bash
cd node
cargo build
cargo test
```

Integration tests include subprocess **two-node**, **restart**, and **three-node fan-out** E2Es (`node/tests/cli_*_e2e.rs`). The `node` binary is at `node/target/debug/node` (or `release` with `--release`).

## Operator runbook (local two-node)

Every machine in a network must use the **same** `genesis.toml` (compare **genesis state commitment** lines when in doubt).

### 1) First node — wallet + genesis for one address

```bash
cd node
cargo run -- init --data-dir ./data-a --genesis-balance 10000000
```

This creates `data-a/wallet.seed`, `data-a/genesis.toml` (one allocation), and prints **Address:** and **Genesis state commitment:**.

### 2) Second node — wallet only (no second genesis file yet)

```bash
cargo run -- init --data-dir ./data-b
```

Note the printed **Address:** for party B.

### 3) Merge allocations into one shared genesis

Edit **one** canonical `genesis.toml` so it contains **both** `[[allocations]]` entries (party A and party B), with the balances you want. You can start from `node/examples/genesis.template.toml`.

Rules:

- Same file (byte-for-byte or same logical allocations → same **state commitment**) on **every** node.
- No duplicate `address` lines; balances must be &gt; 0; addresses must be valid (non-empty, ≤128 chars).

Copy that file to:

- `data-a/genesis.toml`
- `data-b/genesis.toml`

(Overwriting `data-a/genesis.toml` after merge is expected.)

### 4) Run two nodes (two terminals)

**Terminal B** (listener only first — the producer will push blocks here):

```bash
cargo run -- run --data-dir ./data-b --listen 127.0.0.1:9334 --interval-secs 2
```

Copy the line `network: listening on 127.0.0.1:9334` (your port may differ if you use `127.0.0.1:0`).

**Terminal A** (producer + gossip to B):

```bash
cargo run -- run --data-dir ./data-a --listen 127.0.0.1:9333 --peers 127.0.0.1:9334 --interval-secs 2
```

### 5) Submit a transfer

```bash
cargo run -- send --data-dir ./data-a RECEIVER_ADDRESS AMOUNT [FEE]
```

Use **B’s address** as `RECEIVER_ADDRESS`. The tx is queued; **A**’s `run` loop seals blocks and gossips to `--peers`.

### 6) Optional: stricter wall-clock checks on inbound blocks

```bash
cargo run -- run --data-dir ./data-b --peers 127.0.0.1:9333 --max-future-drift-secs 900 ...
```

## Files under a data directory

| File | Role |
|------|------|
| `wallet.seed` | 32-byte secret seed (back up; never commit) |
| `genesis.toml` | Protocol height-0 allocations (must match across the network) |
| `chain.blocks` | Persisted non-genesis blocks |
| `pending_tx.tril` | Queue written by `send`, consumed by `run` |

## CI / tests

- Library and integration tests: `cargo test` from `node/`.
- A **subprocess** two-node test (`tests/cli_two_node_e2e.rs`) spawns the real `node` binary; it runs automatically with `cargo test` and requires no extra setup.

## Scope

V1 is intentionally narrow: **no** smart contracts, **no** fork-choice / reorgs in the current tooling, **no** state snapshots over the wire (nodes replay **blocks** on top of genesis). See [`docs/v1_scope.md`](docs/v1_scope.md).
