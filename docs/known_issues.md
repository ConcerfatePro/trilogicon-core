# Trilogicon — known limitations and honest scope

This file tracks **current** limitations of the reference node and protocol **as shipped**, not obsolete release-candidate task lists. For **what is complete**, see [`v1_checkpoint.md`](v1_checkpoint.md) (V1 core) and [`v2_checkpoint.md`](v2_checkpoint.md) (V2 node hardening).

## What the project is (today)

- A **minimal** base-layer transfer chain in Rust with a **reference node** binary.
- **V1** semantics for validity (blocks, transactions, genesis, linear chain)—see [`v1_scope.md`](v1_scope.md).
- **V2** improvements to **operability**: disk layout and load behavior, restart and pending-queue semantics, peer session and sync bounds, mempool hygiene as **local policy**—see [`v2_scope.md`](v2_scope.md).

## Known limitations (by design or tradeoff)

- **No smart contracts, DeFi, NFTs, bridges, staking/governance** as protocol features.
- **Linear chain only** — no fork choice, reorg, or branch storage in the reference node.
- **State sync** is **replay blocks from shared genesis**, not snapshot download or fast sync.
- **Operator setup** assumes a **shared `genesis.toml`** across nodes that should converge; misconfiguration is a common real-world failure mode (see README troubleshooting).
- **Decentralization / partition safety** — the docs and README explicitly avoid overclaiming; a small network of honest nodes with matching genesis is the happy path the software is built for.
- **DoS and global adversarial networks** — V2 adds **local** bounds (caps, quotas, timeouts); it does not claim resistance to all attack models.
- **Mempool differences across peers** are allowed; only **committed** state must match for the same block sequence.

## V2.1 / later (not bugs—deferred polish)

- **Structured logging** (levels, fields beyond subsystem-tagged stderr) — deferred per [`v2_scope.md`](v2_scope.md).
- **Larger multi-node chaos** and stress matrices — incremental; representative integration and E2E tests exist, not exhaustive fleet coverage.

## Historical note

The branch name **`release/v1.0.0-rc1`** referred to a **V1-era release candidate** that also carried early V2 hardening work merged to `main`. It is **not** the definition of “current stage”; use the checkpoint docs above.

## Repository hygiene (wallet seeds and local `data-*` dirs)

Never commit **`wallet.seed`**, **`chain.blocks`**, or other runtime files from a node data directory. If those files ever reached a **public** remote, treat the material as **compromised** and use fresh keys and directories for anything serious. See [`operator_hygiene.md`](operator_hygiene.md) for conventions; the root `.gitignore` lists patterns to reduce accidental re-adds.

## Contributing fixes vs features

- **Correctness, clarity, docs, tests** aligned with frozen scope are always in scope.
- **Protocol or consensus behavior changes** belong in a **versioned** plan (future V3 or a tagged protocol revision), not as silent drift.
