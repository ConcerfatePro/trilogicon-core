# Trilogicon V3 — Scope (docs-first phase)

**Status:** **Planning and documentation only.** This pass defines V3 **on paper** only. **No** consensus refactor, **no** branch-selection implementation, and **no** change to `append_block` or network commit semantics on `main` until an explicit **implementation approval** plus the checklist in [`v3_test_plan.md`](v3_test_plan.md) is satisfied.

**Frozen baseline:** V1 protocol core and V2 node hardening: [`v1_scope.md`](v1_scope.md), [`v2_scope.md`](v2_scope.md), [`v2_checkpoint.md`](v2_checkpoint.md), [`v2_freeze.md`](v2_freeze.md).

**V3 chain and consensus design (this era):**

| Document | Role |
|----------|------|
| [`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md) | V3 **chain layer**: deterministic validity **vs** ingress/admission policy, compatibility |
| [`fork_choice.md`](fork_choice.md) | **Height-first branch selection** + **temporary** tie-break (specified, not implemented here) |
| [`reorg_model.md`](reorg_model.md) | **Reorg** boundaries, storage design, mempool / old-suffix txs, operator expectations |
| [`finality.md`](finality.md) | **Confirmations** and **integrator-facing** language (**not** PoW/stake/BFT claims) |
| [`v3_test_plan.md`](v3_test_plan.md) | **Tests required** before implementation merges |

---

## Purpose of V3

V3 moves the reference design from **implicit single-tip linearity** (V1/V2: ambiguity avoided by treating competing histories as **rejected** or **not represented**) to an **explicit, written** model for:

1. **Representing** competing valid branches (storage and in-memory policy).
2. **Selecting** which branch is **canonical** for commit and sync using **height-first branch selection** ([`fork_choice.md`](fork_choice.md)) — a **deterministic ordering** over tips, **not** a claim of economic security or optimal sync without further assumptions.
3. **Executing** a **reorg** when selection changes (state rollback/reapply), subject to a **local** automated reorg bound ([`reorg_model.md`](reorg_model.md) §4.1).
4. **Describing** what operators and integrators should assume about **confirmations** ([`finality.md`](finality.md)) without implying security mechanisms the chain does not have.

V3 is **not** a product expansion: **no** smart contracts, DeFi, bridges, NFTs, governance tokens, staking economies, or chain-ID-in-signed-payload unless a **later** protocol version opens that with its own scope doc.

---

## Explicit separation of concerns (V3 mental model)

These layers **must not be conflated** in implementation or docs:

| Layer | Meaning | V3 doc home |
|-------|---------|-------------|
| **Deterministic chain validity** | Given **parent header** **P** and **parent state** **S**, whether block **B** is **valid** per **protocol rules** that are **pure functions** of **block bytes + P + S** (structure, linkage, parent-relative rules **versioned in protocol docs**, ordered tx application). **Excludes** local wall clock and peer identity. | [`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md) §2 |
| **Local ingress / admission policy** | Whether this **node** **attempts** to validate or retain a block **now** (e.g. “too far in the future” vs **local** `now`, resource caps). May differ by deployment; **does not** redefine validity of bytes already admitted. | [`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md) §3 |
| **Height-first branch selection** | Given retained **valid** tips, which tip is **preferred** (height first; **temporary** hash tie-break — see [`fork_choice.md`](fork_choice.md)). **Not** a security proof. | [`fork_choice.md`](fork_choice.md) |
| **Reorg execution** | How the node **mutates** local committed state, disk, and indexes when canonical tip **moves**; bounded by **`MAX_REORG_DEPTH` as local fail-closed policy** ([`reorg_model.md`](reorg_model.md) §4.1). | [`reorg_model.md`](reorg_model.md) |
| **Finality / confirmations** | **Integrator-facing** depth and risk language only. | [`finality.md`](finality.md) |
| **Local peer-defense policy** | Caps, rate limits, disconnects, strikes. Affects **which bytes** are processed. | V2 notes + [`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md) |

---

## `MAX_REORG_DEPTH` classification (resolved)

**`MAX_REORG_DEPTH` is a local fail-closed halt policy** on **automated reorg execution**: if unwinding the **old canonical suffix** would exceed this bound, the node **does not** perform that reorg automatically and remains on the **current committed tip** (with explicit operator signal).

- It is **not** part of **deterministic validity**.
- It is **not** part of **height-first branch selection** inputs (the **abstract** preferred tip may still be computable from retained blocks).
- It is **not** a **protocol-era consensus constant**: peers **need not** agree on the same numeric value; two nodes may **legitimately** have different **committed** tips under the same network view if one refuses a deep rewind. Interoperability expectations must be documented for operators.

A **future protocol version** could define a **normative** depth for **inter-node** behavior; that would be an explicit **consensus / wire** change, not V3 phase 1.

---

## What stays compatible with V1 / V2 (until a written V3 delta says otherwise)

- **Transaction** unsigned payload, hashing, Ed25519 verification, fee burn, nonce rules for **accepted** blocks — unchanged unless a future **protocol version** document amends them.
- **Block** structure, `block_hash` preimage, per-block transaction order for state application — unchanged for V3 phase 1 design; V3 adds **branch selection** among valid forks, not new validity predicates **unless versioned**.
- **Genesis** model and state commitment — unchanged.
- **V2** persistence framing, data-dir binding, pending-tx queue semantics as **local I/O contracts** — remain the reference; V3 **extends** with hash store + metadata per [`reorg_model.md`](reorg_model.md) §9.

**V1/V2 reference node today:** single committed tip, linear extension only, no multi-branch selection. **V3 implementation** will **extend** behavior where specified.

---

## Out of scope (V3)

- Smart contracts, DeFi, NFTs, bridges, app chains, rich governance.
- Claiming **PoW-, stake-, or BFT-grade** safety from height-first selection alone.
- Undocumented consensus or branch-selection behavior on `main`.

---

## Branch selection summary (non-security)

- **Primary:** greater **height** wins among **valid** competing tips ([`fork_choice.md`](fork_choice.md)).
- **Equal height:** **temporary** deterministic tie-break — **grindable**, **not** a security property ([`fork_choice.md`](fork_choice.md) §4).
- **Reorg:** [`reorg_model.md`](reorg_model.md); **local** `MAX_REORG_DEPTH` halts automated execution.
- **Mempool / old-suffix txs:** [`reorg_model.md`](reorg_model.md) §7–8.

---

## Implementation gate (after this docs phase)

1. Review **open questions** in each V3 doc.
2. Promote scope to **implementation approved** (maintainer decision + optional `v3_checkpoint.md` when code ships).
3. Land code **only** with tests from [`v3_test_plan.md`](v3_test_plan.md) and operator-facing notes in the same change train.

---

## V3-08 integration readiness gate

V3 **must not** wire fork-choice, reorg execution, or replay simulation into the live commit path until **all** of the following are satisfied. Until then, `node/src/v3/` remains **inert scaffolding** reviewed against these criteria.

1. **Deterministic block index:** a `BlockIndex`-shaped structure (see `node/src/v3/block_index.rs`) can be rebuilt deterministically from stored canonical chain data (and, when specified, retained side-branch metadata), with tests proving round-trips and failure modes.
2. **Branch selection coverage:** unit tests cover higher height, equal height (tie-break), invalid branch, missing parent, and malformed index inputs for height-first selection ([`fork_choice.md`](fork_choice.md)).
3. **Reorg plan validation:** `ReorgPlan::validate_against_index` in `node/src/v3/reorg_plan.rs` (or its successor) passes on valid rollback/apply suffixes and rejects broken chains, duplicates, and fork placement errors.
4. **Preflight before replay:** `ReorgPreflight` in `node/src/v3/reorg_preflight.rs` rejects structurally valid but locally unsafe plans (for example exceeding `MAX_REORG_DEPTH`-style policy) before any state simulation.
5. **Replay sandbox:** `ReplaySandbox` in `node/src/v3/replay_sandbox.rs` successfully simulates candidate branch application on **cloned** ledger state using canonical validation gates (index linkage, parent-relative timestamps, basic block checks, state transitions), with typed error reporting.
6. **Typed simulation errors:** timestamp policy, basic validation, index linkage, and state transition failures are represented as structured sandbox errors (not only opaque strings), suitable for operator logs and tests.
7. **Storage for side branches:** [`reorg_model.md`](reorg_model.md) (or a follow-on note) specifies how non-canonical blocks are retained, bounded, and evicted on disk.
8. **Mempool after reorg:** [`reorg_model.md`](reorg_model.md) §7–8 behavior is specified and test-backed for transactions invalidated or resurrected by reorg.
9. **Operator / integrator language:** [`finality.md`](finality.md) and README-adjacent docs stay aligned with honest confirmation depth language (no false finality claims).
10. **No silent live integration:** `append_block`, network ingest, storage migration, and CLI must not call V3 integration entrypoints until the checklist above is explicitly signed off.

### Explicit non-goals for V3-08

- No validator economy, staking rewards, or delegated consensus layer.
- No smart contracts, DeFi, bridges, NFTs, or app-chain features.
- No production-grade “finality” or partition-safety claims beyond documented integrator expectations.
- No automatic storage migration or wire-format change unless separately scoped, reviewed, and versioned.
- No network gossip or wire-protocol change for reorgs until a dedicated protocol revision says otherwise.

---

## Relationship to other documents

| Document | Role |
|----------|------|
| [`vision.md`](vision.md) | Long-term direction |
| [`protocol_invariants.md`](protocol_invariants.md) | Global invariants; V3 must preserve §1–8 for a fixed chosen prefix |
| [`modules.md`](modules.md) | Update when commit path or consensus boundaries change |
| [`post_v1_ideas.md`](post_v1_ideas.md) | Backlog; V3 graduates items only via spec + tests |

---

## Honesty

V3 planning does **not** claim Bitcoin-, Ethereum-, or BFT-grade security. **Height-first branch selection** is a **documented ordering** for engineering consistency, not proof of **correct economic consensus**. Prefer **one clear spec** over multiple experimental modes on `main`.
