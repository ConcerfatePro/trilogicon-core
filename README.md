# Trilogicon (TRIL)

Minimal blockchain written in rust.

-----

## Status

- **V1** — first verison that created an outline for accounts, transfers, balances, nonces, and some other stuff
- **V2** — improved the safety and security of V1 which helped fix some bugs and overall improved the project 
- **V3** — currently working on this

-----

## Build

```
cd node
cargo build && cargo test
```

Binary at `node/target/debug/node`.

-----

## Running two nodes locally

both nodes have to share the same genesis file

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

## common issues

**Nodes not syncing** — make sure every node has the same `genesis.toml`.

**Transaction is sent but no block appeared** — `send` only queues the tx. A running `node run` process has to seal it.

**Node quits or exits immediately** — another process is holding a lock onto that data directory. Only one `node run` per data dir.

**Corrupt `chain.blocks`** — V2 fails closed rather than guessing. See `docs/design_notes/v2_persistence_restart.md`.

-----

## Further reading

- `docs/genesis.md` — genesis rules
- `docs/modules.md` — module ownership
- `docs/known_issues.md` — current limitations
- `CONTRIBUTING.md` — fmt, clippy, tests, PR rules
