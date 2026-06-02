# Genesis

Trilogicon has two genesis concepts.

1. **Genesis block:** fixed in code as `Block::genesis()` at height 0. It is not stored in `chain.blocks`.
2. **Genesis state:** loaded from `genesis.toml`, which lists initial account balances.

All nodes on the same network must use the same genesis state.

## File format

```toml
[[allocations]]
address = "ADDR"
balance = 10000000
```

Rules:

- no duplicate addresses;
- address must be non-empty and within the current address length limit;
- balance must be greater than zero;
- allocation order does not change the state commitment.

## Data-dir binding

V2 writes `genesis_bind.toml` after a successful load. On later starts, the node refuses to use the data directory if the binding does not match the current `genesis.toml`.

Do not delete the binding to reuse a data directory with a different genesis after `chain.blocks` has history. Use a fresh data directory or intentionally reset the chain data.

## Commands

Create a wallet and starter genesis allocation:

```bash
cd node
cargo run -- init --data-dir ./data-a --genesis-balance 10000000
```

Use an explicit genesis file:

```bash
cargo run -- run --data-dir ./data-a --genesis ./genesis.toml
```
