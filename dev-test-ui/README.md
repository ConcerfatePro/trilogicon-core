# Trilogicon Dev Test UI

Small localhost-only helper for development. It is not a wallet, explorer, or production interface.

The UI can inspect a data directory, look up account state, and queue test transfers through the local node CLI. It does **not** replace `node run`; keep the real node running in another terminal if you want blocks sealed or synced.

## Run

```bash
cd dev-test-ui
cargo run -- --node-bin ../node/target/debug/node --data-dir ../node/data-a
```

Open the printed localhost URL.

## Notes

- Uses the same local files as the node: `wallet.seed`, `genesis.toml`, `chain.blocks`, and `pending_tx.tril`.
- Intended for throwaway data directories.
- Do not expose it beyond localhost.
