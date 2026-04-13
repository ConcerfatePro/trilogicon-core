# Trilogicon V2 checkpoint

This document closes the **V2 node-hardening** line for the reference implementation: what it was for, what shipped, how we verified it, and what is explicitly **not** claimed. It is a **project gate**, not a security audit or formal verification sign-off.

**Canonical V2 scope:** [`v2_scope.md`](v2_scope.md) (protocol freeze, local vs wire vs consensus, backlog).  
**V1 baseline (unchanged protocol core):** [`v1_checkpoint.md`](v1_checkpoint.md), [`v1_scope.md`](v1_scope.md).

---

## What V2 was meant to accomplish

V2 was defined as **operational robustness** for the **existing linear V1 protocol**: the same block and transaction validity rules, the same genesis model, no fork choice or reorg—only **harder node software** around persistence, restart, TCP peers, linear sync, local mempool policy, and operator-visible behavior.

Goals included: explicit fail-closed recovery semantics, bounded resource use on the network path, testable restart and disk contracts, and documentation that operators can rely on without guessing internal behavior.

---

## What V2 actually delivered

The reference **`node`** crate on `main` implements the V2 backlog described in [`v2_scope.md`](v2_scope.md) **Prioritized implementation backlog**, including:

- **Persistence and `chain.blocks`:** V2 magic + per-record CRC for new files; legacy length-prefixed files still load; load-path validation with **fail closed** on corruption, decode failure, and replay errors. A **narrow** startup repair exists only for **1–3 trailing bytes** after the last **complete** frame (incomplete length-prefix write). A **full** next-frame length prefix with incomplete body **does not** auto-truncate—startup errors. See [`design_notes/v2_persistence_restart.md`](design_notes/v2_persistence_restart.md) and `node/src/storage.rs` (`BlockStore::read_all_blocks_repairing_tail`, `load_blockchain_from_disk`).

- **Restart safety:** Genesis / data-dir binding (`genesis_bind.toml`), exclusive `run` lock (`.node.run.lock`), pending transaction file semantics under advisory lock (`pending_tx.tril`, `.pending_tx.lock`), deterministic replay from disk. Integration coverage: `node/tests/persistence_v2.rs`, `node/tests/restart_matrix_v2.rs`, `node/tests/run_data_dir_lock_e2e.rs`.

- **Networking and session hardening:** Wire handshake (version + genesis commitment), defensive caps, idle/write timeouts, per-session ingress quotas and typed disconnect reasons (`PeerFrameError`, persist failure typing). Design notes: [`design_notes/v2_wire_peer_sync.md`](design_notes/v2_wire_peer_sync.md), [`design_notes/v2_network_defense.md`](design_notes/v2_network_defense.md). Implementation: `node/src/network.rs` and related modules (`peer_book`, `seen`).

- **Synchronization:** Linear catch-up only; peer height **advisory**; bounded work per sync call (`SyncWorkBudget` and tests in `node/tests/v2_hardening.rs`, `node/tests/network_peer_sync.rs`).

- **Mempool (local policy only):** Capacity bounds, FIFO hygiene vs committed ledger, seal-time candidate selection aligned with documented rules (`Mempool::ordered_candidates_for_seal` in `node/src/mempool.rs`; producer policy in `v2_scope.md` and `modules.md`). Hygiene after inbound blocks and after sync catch-up as described in README and `main` / `network` wiring.

- **Operator-facing behavior:** Subsystem-tagged stderr prefixes (`operator_msg`, README *Interpreting stderr*), clearer startup and failure paths for binding, chain load, and pending drain.

- **Merge integration:** Work from `release/v1.0.0-rc1` was merged to `main` and followed by an integration commit that reconciled divergent `main` features (e.g. PeerBook/SeenCache, disk load return type) with V2 modules and tests so the tree is coherent and fully tested.

---

## Stabilization and clarification (final pass)

Before calling V2 closed, the repository underwent a **blocker-remediation** pass:

- **`cargo fmt --all -- --check`** passes.
- **`cargo clippy --all-targets -- -D warnings`** passes.
- **`cargo test`** in `node/` passes (locally and in CI).

Documentation was aligned with **implemented** behavior—notably **mempool producer / seal candidate** rules and **`chain.blocks`** recovery (including the **narrow tail repair** exception vs strict fail closed for other truncation cases).

---

## Automated verification (reference)

CI (`.github/workflows/ci.yml`) on **push/PR to `main`** runs, per OS (**ubuntu-latest**, **windows-latest**, **macos-latest**):

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`

Plus `cargo audit` on Ubuntu. This is the **repository health bar** for the reference node; it does not replace dedicated security review.

---

## Intentionally deferred (V2.1 or later)

From [`v2_scope.md`](v2_scope.md) and honest scoping:

- **Structured logging / log levels** beyond tagged `eprintln!` — listed as **deferred to V2.1** in the V2 backlog.
- **Large-scale multi-node chaos** and exhaustive adversarial fleet testing — incremental backlog; existing tests cover representative paths, not every partition timing.
- **Optional tooling** for operator-driven chain repair (explicit opt-in mutators) — design notes describe manual recovery; automation remains optional future work.

These are **not** blockers for stating that the **V2 scope as documented** is implemented.

---

## What must be true before serious V3 work

V3 (in project vocabulary) implies **new protocol or consensus-era concerns**: e.g. validator sets, fork choice, performance architecture, or other changes **outside** the V2 “same V1 validity, harder node” boundary.

Before treating V3 as the active implementation track:

1. **V2 checkpoint and scope** should remain the agreed baseline (`v2_scope.md` + this file).
2. **Any new consensus-sensitive rule** should be introduced only with an **explicit protocol version** / scope document—not as silent drift in `main`.
3. **Operator and module-boundary docs** (`modules.md`, README, persistence notes) should be updated **with** protocol changes, not after the fact.

V3 **planning** may proceed in docs (`vision.md`, `post_v1_ideas.md`, etc.) without implementing protocol changes.

---

## Honesty

- V2 **does not** prove production decentralization, full DoS resistance against a motivated global adversary, or correctness under all network partitions.
- The node remains a **reference implementation** of a **minimal** chain; V2 makes it **more dependable to run and reason about**, not “complete blockchain product.”
- Wallet GUI, mobile apps, and exchange integrations are **out of scope** for this checkpoint.

---

## Optional developer tooling

The repository may include a **local-only** web shell under `dev-test-ui/` (see `dev-test-ui/README.md`) for **operator convenience** when testing: it is **not** part of the protocol, **not** a production wallet, and **not** covered by the same CI matrix as the `node` crate unless CI is extended. It binds **localhost only** and reuses the `node` library for read-only chain views and pending-queue submission consistent with CLI `send`.

---

## Related documents

| Document | Role |
|----------|------|
| [`v2_scope.md`](v2_scope.md) | Canonical V2 definition and backlog |
| [`design_notes/v2_persistence_restart.md`](design_notes/v2_persistence_restart.md) | Disk artifacts, fail closed, pending semantics |
| [`design_notes/v2_wire_peer_sync.md`](design_notes/v2_wire_peer_sync.md) | Handshake, linear sync |
| [`design_notes/v2_network_defense.md`](design_notes/v2_network_defense.md) | Caps, quotas, timeouts |
| [`modules.md`](modules.md) | Consensus boundary and module ownership |
| [`v1_checkpoint.md`](v1_checkpoint.md) | V1 implementation complete criteria |
| [`v2_freeze.md`](v2_freeze.md) | Post-release V2 maintenance vs protocol changes |
| [`releases/v2.0.0.md`](releases/v2.0.0.md) | V2.0.0 release notes |
| [`v3_scope.md`](v3_scope.md) | V3 planning (consensus era); not implemented until approved |

When this checkpoint matches your judgment of **shipped behavior** on `main`, the project may call **V2 (reference node) technically complete** for the scoped hardening line and point future work to **V2.1** (polish) or **V3** (explicit protocol evolution) as separate decisions.

---

## Release tagging (maintainers)

1. Confirm **pre-release checks** (fmt, clippy `-D warnings`, `cargo test` on `node/`, CI green on `main`).
2. **Recommended tag:** **`v2.0.0`** — V2 is described as **technically complete** for its scope; an `-rc` tag is optional if you want a **public soak** before declaring `v2.0.0`. Use **`v2.0.0-rc1`** only if you intentionally defer “release” language until after feedback; otherwise prefer **`v2.0.0`** and fix issues in **patch tags** (`v2.0.1`) or V2.1 maintenance.
3. Bump **`node/Cargo.toml`** `version` to **`2.0.0`** (or match tag) in the same commit as the tag, or immediately before tagging.
4. Create the annotated tag: `git tag -a v2.0.0 -m "Trilogicon reference node V2.0.0 (node hardening on V1 protocol)"`
5. Push: `git push origin v2.0.0`
6. Point default branch readers at [`releases/v2.0.0.md`](releases/v2.0.0.md) and [`v2_freeze.md`](v2_freeze.md).

After the tag, **V2 protocol semantics** are **frozen** except via **V3** ([`v3_scope.md`](v3_scope.md)) with explicit specs.
