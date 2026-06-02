# Contributing

Trilogicon is intentionally small. Changes should keep the node easier to audit, not make it look bigger than it is.

## Before a PR

```bash
cd node
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

If the change touches the local dev UI, run its checks from `dev-test-ui/` as well.

## Scope rules

- Do not change consensus behavior unless the relevant scope doc says that work is open.
- V2 is node hardening, not a consensus redesign.
- V3 is design/planning until explicitly approved for implementation.
- Do not add smart contracts, staking, governance, DeFi, bridges, or token features as incidental changes.
- Keep operator-facing behavior documented when it changes.

## Code style

- Prefer small, testable changes.
- Use existing module boundaries and error types where possible.
- Keep comments for non-obvious invariants, not for every assignment.
- Avoid broad refactors mixed with behavior changes.

## Docs style

Docs should be plain, technical, and direct. Explain what works, what does not, and how to run or verify it. Avoid whitepaper language.

## Secrets

Never commit `wallet.seed`, test data directories, private keys, local `.env` files, or generated chain data.
