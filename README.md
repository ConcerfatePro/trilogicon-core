# Trilogicon (TRIL)

A minimal blockchain written in Rust. Accounts, signed transfers, blocks, nonces, and TCP sync between nodes. Simple and solid before anything more complex.

-----

## Status

- **V1** — core protocol done (accounts, transfers, balances, nonces)
- **V2** — current, on `main`. Adds restart safety, peer handshakes, CRC-framed chain writes, mempool cleanup
- **V3** — planning only, not active yet

-----

## Build

```
cd node
cargo build && cargo test
```

Binary at `node/target/debug/node`.

-----

## Running two nodes locally

Every node must share the same `genesis.toml`.

```bash
# Initialize both nodes
cargo run -- init --data-dir ./data-a --genesis-balance 10000000
cargo run -- init --data-dir ./data-b

# Edit genesis.toml to include both addresses, copy to both data dirs

# Start node B first, then A
cargo run -- run --data-dir ./data-b --listen 127.0.0.1:9334 --interval-secs 2
cargo run -- run --data-dir ./data-a --listen 127.0.0.1:9333 --peers 127.0.0.1:9334 --interval-secs 2

# Send a transfer (queues it — a running node seals it into a block)
cargo run -- send --data-dir ./data-a <B_ADDRESS> 100 1
```

Running solo? Skip `--peers`.

-----

## Data directory

|File             |Purpose                                      |
|-----------------|---------------------------------------------|
|`wallet.seed`    |Secret seed — never commit this              |
|`genesis.toml`   |Must match across all nodes                  |
|`chain.blocks`   |Stored blocks                                |
|`pending_tx.tril`|Tx queue written by `send`, consumed by `run`|

-----

## Common issues

**Nodes won’t sync** — make sure every node has the same `genesis.toml`.

**Transaction sent but no block appeared** — `send` only queues the tx. A running `node run` process has to seal it.

**Node exits immediately** — another process holds the lock on that data directory. Only one `node run` per data dir.

**Corrupt `chain.blocks`** — V2 fails closed rather than guessing. See `docs/design_notes/v2_persistence_restart.md`.

-----

## Further reading

- `docs/genesis.md` — genesis rules
- `docs/modules.md` — module ownership
- `docs/known_issues.md` — current limitations
- `CONTRIBUTING.md` — fmt, clippy, tests, PR rules