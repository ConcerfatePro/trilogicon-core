# Trilogicon

Trilogicon is a small account-based blockchain written in Rust. It is a reference implementation, not a production network.

It supports signed transfers, nonces, block validation, deterministic replay from disk, shared genesis files, local persistence, and basic TCP sync between nodes.

## Status

- **V1:** protocol core is complete for its defined scope. See [`docs/v1_scope.md`](docs/v1_scope.md) and [`docs/v1_checkpoint.md`](docs/v1_checkpoint.md).
- **V2:** reference-node hardening is complete for its scope: persistence, restart behavior, bounded peer sync, mempool cleanup, and operator diagnostics. See [`docs/v2_scope.md`](docs/v2_scope.md) and [`docs/v2_checkpoint.md`](docs/v2_checkpoint.md).
- **V3:** planning/design work only. V3 modules under `node/src/v3/` are inert and are not wired into live consensus, storage, networking, or CLI behavior. See [`docs/v3_scope.md`](docs/v3_scope.md).

Trilogicon does **not** currently provide smart contracts, staking, validator economics, production finality, fork-choice repair in the live node, bridges, DeFi, NFTs, or a finished public-network security model.

## Build and test

```
cd node
cargo build
cargo test
cargo clippy -- -D warnings
```

The main binary is `node/target/debug/node` unless built with `--release`.

## Local two-node run

Create the first wallet and a genesis file:

```bash
cargo run -- init --data-dir ./data-a --genesis-balance 10000000
```

Create a second wallet:

```bash
cargo run -- init --data-dir ./data-b
```

Merge both addresses into one shared `genesis.toml`, then copy the same final file into both data directories:

```text
data-a/genesis.toml
data-b/genesis.toml
```

Start node B:

```bash
cargo run -- run --data-dir ./data-b --listen 127.0.0.1:9334 --interval-secs 2
```

Start node A and connect it to B:

```bash
cargo run -- run --data-dir ./data-a --listen 127.0.0.1:9333 --peers 127.0.0.1:9334 --interval-secs 2
```

Send from A to B:

```bash
cargo run -- send --data-dir ./data-a <B_ADDRESS> 100 1
```

`send` only queues the transaction in `pending_tx.tril`. A running `node run` process drains the queue, seals a block, persists it, and gossips it.

Running solo? Omit `--peers`.

## Data directory

| File | Meaning |
|------|---------|
| `wallet.seed` | 32-byte wallet secret. Do not commit it. |
| `genesis.toml` | Shared height-0 allocations. Must match across nodes. |
| `genesis_bind.toml` | V2 data-dir binding to the genesis state commitment. Mismatch refuses startup. |
| `.node.run.lock` | Prevents two `node run` processes from sharing one data dir. |
| `chain.blocks` | Persisted non-genesis blocks. New files use V2 magic + CRC framing; legacy files still load. |
| `pending_tx.tril` | Transactions queued by `send` and drained by `run`. |

If `chain.blocks` is corrupt, the node refuses startup instead of guessing. See [`docs/design_notes/v2_persistence_restart.md`](docs/design_notes/v2_persistence_restart.md).

## Common issues

- `wallet.seed already exists`: `init` will not overwrite an existing wallet. Use a fresh data directory for tests.
- `genesis file not found`: provide `--genesis PATH` or place `genesis.toml` in the data directory.
- Nodes do not match: check that all nodes use the same merged `genesis.toml` and genesis state commitment.
- `send` succeeded but no block appears: make sure `node run` is running and sealing is enabled.
- Peer handshake fails: genesis commitments or wire versions do not match.

## More docs

- [`docs/protocol_overview.md`](docs/protocol_overview.md) - V1 ledger and block rules.
- [`docs/genesis.md`](docs/genesis.md) - genesis block, genesis state, and data-dir binding.
- [`docs/modules.md`](docs/modules.md) - where core behavior lives in the Rust code.
- [`docs/known_issues.md`](docs/known_issues.md) - current limitations.
- [`docs/wire_protocol.md`](docs/wire_protocol.md) - TCP message framing.
- [`dev-test-ui/README.md`](dev-test-ui/README.md) - local browser helper for development.

## Contributing

Run formatting, tests, and clippy before opening a PR:

```bash
cd node
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the rest.
