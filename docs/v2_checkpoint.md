# V2 Checkpoint

V2 is complete for its intended reference-node hardening scope.

## Shipped behavior

- V2 `chain.blocks` magic + CRC for new files.
- Legacy chain files still load.
- Fail-closed load on corruption, decode failure, CRC mismatch, or replay failure.
- Narrow incomplete-tail repair only where replay equivalence is clear.
- Data-dir genesis binding.
- Safer pending transaction drain/rewrite.
- TCP handshake with wire version and genesis commitment.
- Peer/session caps, stale/invalid block budgets, and typed disconnect reasons.
- Linear sync with bounded work.
- Mempool capacity and committed-ledger hygiene.
- Tagged stderr for startup, storage, sync, peer, mempool, seal, and pending paths.

## Tests

Run:

```bash
cd node
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

Important coverage lives in storage unit tests, `persistence_v2`, `restart_matrix_v2`, network defense/sync tests, and CLI E2Es.

## Release tagging

For maintainers:

```bash
git tag -a v2.0.0 -m "Trilogicon v2.0.0"
git push origin v2.0.0
```

After tagging, use [`v2_freeze.md`](v2_freeze.md) for what can still change under the V2 name.
