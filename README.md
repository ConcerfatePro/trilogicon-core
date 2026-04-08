# Trilogicon (TRIL)

Minimal V1 value-transfer chain in Rust: accounts, signed transfers, blocks, deterministic replay protection, shared **genesis**, and basic TCP sync between nodes.

## Current release-candidate phase

The current stabilization branch is:

`release/v1.0.0-rc1`

V1 is feature-frozen. The current focus is bug fixing, testing, documentation, operator experience, and private technical validation. For limits of what V1 **promises** versus honest-node validation only, see [What V1 does not guarantee](docs/v1_scope.md#what-v1-does-not-guarantee) in `docs/v1_scope.md`.

Design rules live under [`docs/`](docs/) (scope, invariants, architecture). **Genesis** is documented in [`docs/genesis.md`](docs/genesis.md). Module ownership and the consensus boundary are summarized in [`docs/modules.md`](docs/modules.md); the V1 completion checklist is [`docs/v1_checkpoint.md`](docs/v1_checkpoint.md). For fmt/clippy/test and PR expectations, see [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Troubleshooting

### `wallet.seed already exists`
`init` will not overwrite an existing wallet. Use a fresh data directory or remove the old test directory first.

### `genesis file not found`
The node expects `genesis.toml` in the data directory unless `--genesis PATH` is provided. All nodes in the same network must use the same shared genesis.

### Nodes do not match
Check that both nodes use the same merged `genesis.toml`. Compare genesis state commitment if needed.

### `send` succeeded but no block appeared
`send` only queues the transaction. A running `run` process must pick it up, seal a block, and gossip it to peers.

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

Then on the running node process (stderr uses a `tril:<area>:` prefix), something like:

```text
tril:mempool: accepted tx
tril:produce: sealed height=1 txs=1
```

With `--peers`, the node also attempts catch-up sync to those addresses after each block interval (see `tril:sync:` lines when new blocks arrive).

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
| `peer_book.toml` | V2: known peer addresses, failure streaks, last success time (updated by `run`) |

### V2 networking flags (`run`)

All peers are still untrusted; these options harden **how** you connect, not ledger rules.

- `--network-id N` — logical network id (default `1`); must match across nodes that should speak to each other.
- `--handshake` — send a v2 **HELLO** (wire version, `network_id`, raw genesis commitment, tip) before block/tx/sync traffic on **outbound** connections.
- `--require-handshake-inbound` — first inbound frame must be **HELLO** (strict; incompatible with old peers that send `GET_BLOCKS` first).
- `--no-legacy-inbound` — disallow legacy first frames when not requiring HELLO (use with care).
- `--exchange-peers` — after each successful block sync to a peer, request a capped peer list (`REQUEST_PEERS` / `PEERS`) and merge it into the in-memory book (still persisted to `peer_book.toml` on the same cadence as sync). Off by default so simple test servers are not surprised by extra opcodes.
- `--announce-blocks` — gossip sealed blocks with `BLOCK_INV` first; the peer may answer `BLOCK_WANT` to receive the full `BLOCK` body. Off by default (full-block push remains the default).

`run` merges `--peers` into `peer_book.toml`, uses cooldowns after repeated failures, and refreshes the book after sync rounds. Inbound v2 **HELLO** peers are recorded for health/cooldown tracking but are **not** put on the `OP_PEERS` list (so ephemeral client ports are not gossiped). If `chain.blocks` ends with a truncated frame on startup, the tail is dropped to the last complete block and `tril:storage:` logs a repair line.

Each inbound TCP session stops after **8192** framed messages (`MAX_FRAMES_PER_INBOUND_SESSION` in `network.rs`) to cap per-connection work.

Catch-up sync (`sync_from_peer`) applies at most **`MAX_BLOCKS_APPLIED_PER_SYNC`** blocks per call (**262144**, an operational cap below `MAX_BLOCKS_APPLIED_PER_SYNC_WIRE_MAX` = rounds × batch), alongside per-batch (`4096`) and per-round (`256`) limits — see [`docs/wire_protocol.md`](docs/wire_protocol.md).

## CI / tests

- **GitHub Actions** (`.github/workflows/ci.yml`): on each push/PR to `main`, **`cargo fmt --check`**, **`cargo clippy -D warnings`**, and **`cargo test`** run on **Ubuntu**, **Windows**, and **macOS**. A separate job runs **`cargo audit`** (RustSec advisory DB) against `node/Cargo.lock`.
- **Dependabot** (`.github/dependabot.yml`) opens weekly PRs to bump **GitHub Actions** dependencies.
- From repo root, **`make ci`** runs the same Cargo checks as CI (see [`CONTRIBUTING.md`](CONTRIBUTING.md)).
- Toolchain: stable Rust with `rustfmt` + `clippy` (`rust-toolchain.toml` at repo root).
- Library and integration tests: `cargo test` from `node/`
- A subprocess two-node test (`tests/cli_two_node_e2e.rs`) spawns the real `node` binary and runs automatically with `cargo test`
- Restart and three-node fan-out E2Es also run under `cargo test`
- `tests/cli_gossip_extensions_e2e.rs` covers `--handshake` + `--exchange-peers` + `--announce-blocks`

## Scope

V1 is intentionally narrow:

- **no** smart contracts
- **no** fork-choice / reorg tooling in the current operator flow
- **no** state snapshots over the wire

Nodes replay persisted **blocks** on top of shared genesis. See [`docs/v1_scope.md`](docs/v1_scope.md) (including [non-goals](docs/v1_scope.md#v1-non-goals) and [what V1 does not guarantee](docs/v1_scope.md#what-v1-does-not-guarantee)). TCP framing and opcodes: [`docs/wire_protocol.md`](docs/wire_protocol.md).

### CLI (`node` binary)

Matches `Usage` from `node/src/main.rs` (abridged):

- `init [--data-dir DIR] [--genesis-balance N]`
- `run [--data-dir DIR] [--genesis PATH] [--interval-secs SECS] [--listen HOST:PORT] [--peers A,B,...] [--max-future-drift-secs N] [--network-id N] [--handshake] [--require-handshake-inbound] [--no-legacy-inbound] [--exchange-peers] [--announce-blocks]`
- `send [--data-dir DIR] [--genesis PATH] RECEIVER AMOUNT [FEE]`

Default genesis path is `{data-dir}/genesis.toml`. Data-dir files are summarized in [Files under a data directory](#files-under-a-data-directory).
