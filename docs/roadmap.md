# Roadmap

## Done for current scope

- V1 transfer protocol: accounts, balances, nonces, signed transactions, blocks, validation, shared genesis.
- V2 node hardening: persistence checks, restart behavior, peer/session bounds, linear sync, mempool hygiene, and clearer operator messages.

## Current design work

- V3 branch/index/reorg planning.
- Deterministic block-index rebuild spec.
- Side-branch storage design.
- Mempool-after-reorg policy.
- Operator/finality wording for future reorg-aware behavior.

## Later, only with explicit scope

- Chain ID in signed payloads.
- Fee routing or rewards.
- Validator/staking model.
- Production network security work.
- Smart contracts or application-layer features.

The project should stay small enough that one developer can audit the behavior end to end.
