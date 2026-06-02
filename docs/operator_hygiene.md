# Operator Hygiene

Use fresh data directories when testing different networks or genesis files.

## Keep separate

- one data directory per node;
- one shared `genesis.toml` per network;
- no committed `wallet.seed` files;
- no casual reuse of `chain.blocks` with a different genesis.

## When startup refuses

Do not work around a refusal by deleting files at random.

- Genesis mismatch: use the correct genesis or a fresh data directory.
- Corrupt `chain.blocks`: restore, repair intentionally, or reset and resync.
- Run lock held: stop the existing `node run` process or use another data directory.

## Shared deployments

If nodes are expected to stay on one chain, standardize consensus-sensitive CLI values such as `--max-future-drift-secs`.
