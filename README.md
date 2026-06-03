# Trilogicon

Trilogicon is a small account-based blockchain written in Rust.

This repo contains the core node/reference implementation. It is useful for local testing, protocol work, and understanding how the chain behaves. It is **not** a production network.

The chain currently handles the basics:

* accounts and balances
* signed transfers
* nonces for replay protection
* block creation and validation
* deterministic replay from disk
* shared genesis files
* local persistence
* basic TCP sync between nodes

The goal is to keep the base layer understandable before adding anything larger on top.

## Current state

### V1 — protocol core

V1 is complete for its original scope.

It covers the ledger rules: accounts, signed transfers, nonces, blocks, shared genesis, and deterministic validation.

Read more:

* [`docs/v1_scope.md`](docs/v1_scope.md)
* [`docs/v1_checkpoint.md`](docs/v1_checkpoint.md)

### V2 — node hardening

V2 is complete for its original scope.

It made the reference node safer to run locally by improving persistence, restart behavior, peer sync limits, mempool cleanup, diagnostics, and storage handling.

Read more:

* [`docs/v2_scope.md`](docs/v2_scope.md)
* [`docs/v2_checkpoint.md`](docs/v2_checkpoint.md)

### V3 — design work

V3 is still planning/design work.

Some V3 code exists under `node/src/v3/`, but it is inert. It is not wired into live consensus, storage, networking, or CLI behavior yet.

Read more:

* [`docs/v3_scope.md`](docs/v3_scope.md)

## What this is not

Trilogicon does not currently include:

* smart contracts
* staking
* validator economics
* production finality
* live fork-choice repair
* bridges
* DeFi
* NFTs
* a finished public-network security model

Those are intentionally out of scope for now.

## Build and test

From the repo root:

```bash
cd node
cargo build
cargo test
cargo clippy -- -D warnings
```

The debug binary is created at:

```text
node/target/debug/node
```

For a release build:

```bash
cargo build --release
```

## Running a local node

The node stores its wallet, genesis file, pending transactions, and block data inside a data directory.

For quick testing, use a fresh directory:

```bash
cargo run -- init --data-dir ./data-a --genesis-balance 10000000
```

This creates a wallet and a local genesis file for node A.

To run the node:

```bash
cargo run -- run --data-dir ./data-a --listen 127.0.0.1:9333 --interval-secs 2
```

Running by yourself? You can stop here. No peers are required for a solo local run.

## Running two local nodes

For sync testing, create two data directories.

First, create node A:

```bash
cargo run -- init --data-dir ./data-a --genesis-balance 10000000
```

Then create node B:

```bash
cargo run -- init --data-dir ./data-b
```

Each node has its own wallet. For them to sync, both nodes need the same final `genesis.toml`.

Merge both wallet addresses into one shared genesis file, then copy that same file into both directories:

```text
data-a/genesis.toml
data-b/genesis.toml
```

The genesis files must match. If they do not, the nodes will reject each other.

Start node B first:

```bash
cargo run -- run --data-dir ./data-b --listen 127.0.0.1:9334 --interval-secs 2
```

In another terminal, start node A and connect it to B:

```bash
cargo run -- run --data-dir ./data-a --listen 127.0.0.1:9333 --peers 127.0.0.1:9334 --interval-secs 2
```

Now send a transaction from A to B:

```bash
cargo run -- send --data-dir ./data-a <B_ADDRESS> 100 1
```

Replace `<B_ADDRESS>` with node B’s wallet address.

One important detail: `send` does not directly seal a block. It queues the transaction in `pending_tx.tril`. A running `node run` process drains the queue, seals a block, persists it, and gossips it to peers.

## Data directory files

| File                | What it is                                                                                   |
| ------------------- | -------------------------------------------------------------------------------------------- |
| `wallet.seed`       | The node wallet secret. Do not commit this.                                                  |
| `genesis.toml`      | Shared height-0 balances. This must match across nodes.                                      |
| `genesis_bind.toml` | V2 binding between the data directory and the genesis state commitment.                      |
| `.node.run.lock`    | Prevents two `node run` processes from using the same data directory.                        |
| `chain.blocks`      | Persisted non-genesis blocks. New files use V2 magic + CRC framing. Legacy files still load. |
| `pending_tx.tril`   | Transactions queued by `send` and later drained by `run`.                                    |

If `chain.blocks` is corrupt, the node refuses to start instead of guessing. That behavior is intentional.

More detail:

* [`docs/design_notes/v2_persistence_restart.md`](docs/design_notes/v2_persistence_restart.md)

## Common issues

### `wallet.seed already exists`

`init` will not overwrite an existing wallet.

Use a new data directory if you want a clean test.

### `genesis file not found`

Place `genesis.toml` in the data directory, or pass one explicitly with:

```bash
--genesis PATH
```

### Nodes connect but do not agree

Check that every node is using the same merged `genesis.toml`.

Also check that the genesis state commitment matches. V2 binds the data directory to the expected genesis state, and startup will fail if the binding does not match.

### `send` worked, but no block appeared

Make sure `node run` is running.

`send` only writes to the pending transaction queue. The running node process is what drains the queue and seals blocks.

### Peer handshake fails

The nodes likely disagree on genesis commitment, wire version, or protocol expectations.

Start by checking the genesis files.

## More docs

Start here:

* [`docs/protocol_overview.md`](docs/protocol_overview.md) — V1 ledger and block rules
* [`docs/genesis.md`](docs/genesis.md) — genesis block, genesis state, and data-dir binding
* [`docs/modules.md`](docs/modules.md) — where the main Rust code lives
* [`docs/known_issues.md`](docs/known_issues.md) — current limitations
* [`docs/wire_protocol.md`](docs/wire_protocol.md) — TCP message framing
* [`dev-test-ui/README.md`](dev-test-ui/README.md) — local browser helper for development

## Contributing

Before opening a PR, run:

```bash
cd node
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

Then read:

* [`CONTRIBUTING.md`](CONTRIBUTING.md)

Keep changes small when possible. Trilogicon is easier to reason about when each change has a clear purpose.
