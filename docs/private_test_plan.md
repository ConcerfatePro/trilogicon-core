# Private Test Plan

Use this before tagging or sharing a build.

```bash
cd node
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

Manual smoke test:

1. Create two fresh data directories.
2. Build one shared `genesis.toml` with both addresses.
3. Run two nodes locally.
4. Send one transfer.
5. Stop and restart both nodes.
6. Confirm height and balances reload consistently.

Also test one failure path: wrong genesis, corrupt chain file, or second `node run` on the same data directory.
