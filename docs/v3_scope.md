# Trilogicon V3 — Scope and planning (planning phase only)

**Status:** **Planning and documentation only.** No V3 consensus or protocol behavior is implemented on `main` until this scope (or a successor revision) is explicitly promoted to **“implementation approved”** and paired with a **protocol delta** document. Until then, treat V3 as **design discipline**, not a roadmap promise of shipped features.

**Frozen baseline:** V1 protocol core + V2 node hardening as documented in [`v1_scope.md`](v1_scope.md), [`v2_scope.md`](v2_scope.md), [`v2_checkpoint.md`](v2_checkpoint.md), and [`v2_freeze.md`](v2_freeze.md).

---

## Purpose of V3

V3 exists to move Trilogicon from a **single linear tip** with **no fork choice** to a **clearer, explicitly specified** model for:

- how multiple candidate histories are **represented** (if at all),
- how a node **selects** which chain to treat as authoritative,
- what **reorganization** (if any) means for committed state and operators,
- how **confirmations** or **finality** are described (even if weak at first),

without turning the project into a smart-contract platform, wallet product, or ecosystem grab-bag.

V3 is **not** primarily about throughput marketing or feature count; it is about **consensus and chain semantics clarity** and a defensible path toward **stronger decentralization assumptions**—only where honestly achievable.

---

## Why V3 exists after V2

- **V1** fixed the **value-transfer protocol** (tx/block validity, genesis, linear extension).
- **V2** fixed **node reliability** on that same protocol (persistence, sync, defense, mempool hygiene) **without** changing validity rules or adding fork choice.
- **V3** addresses the **intentional gap** left by V1/V2: real networks see **divergent views** of history; a linear-only reference node avoids the problem by **rejecting** ambiguity rather than **resolving** it. V3 is where the project may **define resolution rules**—carefully, in writing, before code.

---

## In scope (V3)

Subject to later tightening, V3 **may** include:

| Area | Examples (not commitments) |
|------|------------------------------|
| **Chain selection** | Longest-work / heaviest chain, GHOST-like variants, or other **documented** rules—chosen explicitly, not by accident. |
| **Reorg semantics** | Max depth, handling of user-facing confirmations, what happens to mempool vs committed state under reorg. |
| **Fork handling** | Storing or pruning alternate tips; safe rollback of state; **no silent** divergence between nodes. |
| **Wire / session versioning** | Negotiating a **protocol era** so V1/V2-only nodes can **refuse** or **degrade** safely vs V3-capable peers. |
| **Confirmation / finality language** | Operator- and integrator-facing definitions (even if “probabilistic” or weak at first). |
| **Performance tied to consensus path** | Batch validation, structures that support reorg—**only** where they do not smuggle in undefined fork choice. |

All of the above require **written specification** before implementation is merged to `main` as **active** behavior.

---

## Out of scope (V3)

- **Smart contracts**, DeFi primitives, NFTs, bridges, app chains.
- **Wallet / exchange / custody productization** (GUI, “official wallet,” key management UX beyond what the reference CLI already does).
- **Rich genesis** (e.g. arbitrary contracts at genesis), **chain ID in signed payloads**, **governance tokens**, **staking economies**—unless pulled into a **later** explicit protocol version with its own scope doc.
- **Undocumented consensus changes** on `main` (anything that changes what blocks commit without an updated protocol doc).
- **Speculative mempool behavior** that changes **which valid transactions appear in blocks** vs a reference V1/V2 node unless called out as **consensus-visible** and versioned.

---

## What stays frozen from V2 (until V3 supersedes explicitly)

Until a **V3 protocol delta** is published and implementation is enabled:

- **V1 transaction and block validity** (payloads, signatures, nonce rules for **accepted** blocks) remain as today unless the V3 spec **explicitly** amends them.
- **V2 operator and disk contracts** (`chain.blocks` framing, binding, pending queue semantics) remain the default **reference** behavior for “V2-era” deployments; V3 may **extend** storage or add migration, not silently break existing nodes without documentation.
- **Classification** in [`modules.md`](modules.md): anything that changes **committed state** must remain traceable through explicit APIs and docs.

---

## What counts as protocol-changing work

Any change that affects **either**:

1. **Inter-node agreement** on the canonical history after the same honest messages, or  
2. **Which blocks or transactions a node may commit** under the same genesis,

is **protocol-changing**. Examples: new fork-choice rule, different block acceptance order, new fields in signed tx payloads, different fee destination, altered timestamp validity.

**Not** protocol-changing by default: logging, local caps, disk layout **if** replay still yields identical committed state for the same block sequence, pure refactors, optional **non-committing** tooling.

Gray areas require an **explicit decision** recorded in the V3 design record (ADR or protocol delta), not merge-first judgment calls.

---

## First V3 work batch (recommended order)

**Batch A — Planning only (this document + artifacts; no behavior change on `main`)**

1. **Lock V2** with a git tag and [`v2_freeze.md`](v2_freeze.md) (see release process).
2. **Publish** [`releases/v2.0.0.md`](releases/v2.0.0.md) (or chosen tag) so operators know what “V2” means.
3. **Draft** `docs/protocol_v3_chain_rules.md` (or equivalent) as an **ADR-style** chain-selection and reorg document: assumptions, adversary model (honest but asynchronous? Byzantine?), and **non-goals**.
4. **List invariants** that any V3 design must preserve (e.g. deterministic state given a chosen canonical prefix).

**Batch B — First implementation (only after Batch A is reviewed)**

1. **Wire / version negotiation** scaffolding that allows **explicit disconnect** or **“unknown era”** between peers—**default off** or **compatible with V2** until spec says otherwise.
2. **Library-only** structures for blocks indexed by height/hash **without** switching `Blockchain` commit path—**tests only** or behind a feature flag **default off**.
3. **No change** to `append_block` / network acceptance until the spec is merged and checklist-signed.

**Batch C — Later (out of scope until B is done)**

- State migration tools, snapshot sync, validator sets, production hardening.

---

## What must be tested before V3 work is “landed”

**Planning phase:** N/A—docs only.

**Per implementation merge** (when code appears):

- **Unit tests** for any pure fork-choice or scoring function (synthetic DAGs, tie-breaks).
- **Integration tests** for reorg depth limits, mempool behavior under reorg, and **no regression** on V1/V2 rejection matrices where still applicable.
- **Multi-node** scenarios: partition, heal, and **explicit** expected tip—documented in test names or `docs/`.
- **CI** on all platforms the project supports (same bar as V2).
- **Operator doc update** in the same PR as behavior change.

V3 is **not landed** until tests and docs match the spec, not the other way around.

---

## Relationship to other documents

| Document | Role |
|----------|------|
| [`vision.md`](vision.md) | Long-term ladder; V3 one paragraph summary |
| [`v2_freeze.md`](v2_freeze.md) | What stops changing in the V2 line after release |
| [`modules.md`](modules.md) | Consensus boundary—must be updated when commit path changes |
| [`post_v1_ideas.md`](post_v1_ideas.md) | Backlog; V3 items should graduate here or into ADRs before coding |

---

## Honesty

- V3 planning does **not** commit the project to shipping PoS, BFT, or Bitcoin-grade security on day one.
- **Decentralization** claims must stay proportional to what is actually specified and tested.
- Prefer **one clear spec** over multiple half-implemented modes.

When V3 implementation is ready to start, add a **`v3_checkpoint.md`** (mirror V1/V2) and a **protocol version** field in documentation that operators can read.
