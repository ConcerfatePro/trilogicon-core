# Trilogicon (TRIL)

Minimal V1 value-transfer chain in Rust: accounts, signed transfers, blocks, deterministic replay protection, shared **genesis**, and basic TCP sync between nodes.

Design rules live under [`docs/`](docs/) (scope, invariants, architecture). **Genesis** is documented in [`docs/genesis.md`](docs/genesis.md). Module ownership and the consensus boundary are summarized in [`docs/modules.md`](docs/modules.md); the V1 completion checklist is [`docs/v1_checkpoint.md`](docs/v1_checkpoint.md). For fmt/clippy/test and PR expectations, see [`CONTRIBUTING.md`](CONTRIBUTING.md).

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

This creates:

- `data-a/wallet.seed`
- `data-a/genesis.toml` (initially with one allocation)

It also prints:

- **Address:**
- **Genesis state commitment:**

Note:
- `wallet.seed` is secret and should never be committed.
- If `wallet.seed` already exists in that data directory, `init` will refuse to overwrite it. Use a fresh directory or remove the old test directory first.

### 2) Second node — wallet only

```bash
cargo run -- init --data-dir ./data-b
```

This creates:

- `data-b/wallet.seed`

Note the printed **Address:** for party B.

### 3) Merge allocations into one shared genesis

Edit **one** canonical `genesis.toml` so it contains **both** `[[allocations]]` entries (party A and party B), with the balances you want. You can start from `node/examples/genesis.template.toml`.

Rules:

- Use the same final file on **every** node.
- Same logical allocations should produce the same **genesis state commitment**.
- No duplicate `address` lines.
- Balances must be `> 0`.
- Addresses must be valid (non-empty, `<= 128` chars).

Copy that merged file to:

- `data-a/genesis.toml`
- `data-b/genesis.toml`

Overwriting `data-a/genesis.toml` after merge is expected.

Important:
- If you manually edit a genesis file that already contains an old printed commitment comment, remove that old comment unless it has been regenerated to match the new merged allocations.

### 4) Run two nodes (two terminals)

**Terminal B** (listener first):

```bash
cargo run -- run --data-dir ./data-b --listen 127.0.0.1:9334 --interval-secs 2
```

You should see something like:

```text
network: listening on 127.0.0.1:9334
Trilogicon node | height=0 | wallet=...
```

**Terminal A** (producer + gossip to B):

```bash
cargo run -- run --data-dir ./data-a --listen 127.0.0.1:9333 --peers 127.0.0.1:9334 --interval-secs 2
```

You should see something like:

```text
network: listening on 127.0.0.1:9333
Trilogicon node | height=0 | wallet=...
```

### 5) Submit a transfer

```bash
cargo run -- send --data-dir ./data-a RECEIVER_ADDRESS AMOUNT [FEE]
```

Use **B’s address** as `RECEIVER_ADDRESS`.

Example:

```bash
cargo run -- send --data-dir ./data-a <B_ADDRESS> 100 1
```

`send` does **not** directly create a block. It writes the transaction to `pending_tx.tril`. The running `run` loop on node A later picks it up, seals it into a block, and gossips that block to peers.

You should see output similar to:

```text
Queued tx ... -> RECEIVER amount 100 fee 1 (nonce 0)
```

Then on the running node process, something like:

```text
mempool: accepted tx
sealed height=1 with 1 transaction(s)
```

### 6) Optional: stricter wall-clock checks on inbound blocks

Example:

```bash
cargo run -- run --data-dir ./data-b --listen 127.0.0.1:9334 --peers 127.0.0.1:9333 --interval-secs 2 --max-future-drift-secs 900
```

This can be useful when testing stricter acceptance of future-dated inbound blocks.

### 7) Optional: restart sanity check

After at least one successful transfer:

- Stop both nodes with `Ctrl+C`
- Restart them with the same commands
- Confirm they reload the same height from disk

If restart worked correctly, both nodes should come back with the same persisted height and remain in sync.

## Files under a data directory

| File | Role |
|------|------|
| `wallet.seed` | 32-byte secret seed (back up; never commit) |
| `genesis.toml` | Protocol height-0 allocations (must match across the network) |
| `chain.blocks` | Persisted non-genesis blocks |
| `pending_tx.tril` | Queue written by `send`, consumed by `run` |

## CI / tests

- Library and integration tests: `cargo test` from `node/`
- A subprocess two-node test (`tests/cli_two_node_e2e.rs`) spawns the real `node` binary and runs automatically with `cargo test`
- Restart and three-node fan-out E2Es also run under `cargo test`

## Scope

V1 is intentionally narrow:

- **no** smart contracts
- **no** fork-choice / reorg tooling in the current operator flow
- **no** state snapshots over the wire

Nodes replay persisted **blocks** on top of shared genesis. See [`docs/v1_scope.md`](docs/v1_scope.md).
