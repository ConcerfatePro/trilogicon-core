# Trilogicon V3 — Confirmations and integrator-facing depth

**Status:** design document. **Does not** change node behavior in this docs-only phase. **Does not** redefine **deterministic chain validity** ([`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md)).

**Reads with:** [`fork_choice.md`](fork_choice.md), [`reorg_model.md`](reorg_model.md).

---

## 1. Purpose

Separate ideas that are easy to conflate:

| Concept | Meaning |
|---------|---------|
| **Valid (chain rules)** | Block passes **§2** predicates for some parent **P**/state **S** — **no** wall clock. |
| **Canonical (this node)** | Block sits on the **committed** chain **after** branch selection **and** successful reorg execution **subject to** local `MAX_REORG_DEPTH` ([`reorg_model.md`](reorg_model.md)). |
| **Integrator “settled”** | Off-chain **business** choice to wait **N** confirmations before acting — **not** a protocol proof. |

This document uses **“finality”** only as **informal** shorthand for **integrator risk posture**. It does **not** denote **BFT finality**, **proof-of-work settlement**, **proof-of-stake economic finality**, or any **on-chain gadget** V3 does not implement.

---

## 2. Confirmations (depth-based, **relative to committed tip**)

For block **B** at height **h** on the **current committed** canonical chain, tip **T** at **H(T)**:

`confirmations(B) = H(T) - h + 1` (adjust **±1** in implementation to match exact height convention for genesis).

**Meaning:** how many **block layers** sit above **B** on **this node’s** committed chain **right now**.

**Not a guarantee:** [`fork_choice.md`](fork_choice.md) may **prefer** another tip; [`reorg_model.md`](reorg_model.md) §4.1 may **block** switching; **B** can leave the committed chain if a reorg occurs.

---

## 3. What deeper confirmations do **not** imply

**Do not** interpret deeper confirmations as:

- **Proof** that a **global** majority agrees with this tip (no such layer is specified).
- **Work accumulation** or **stake slashing** backing (no such model in V3 phase 1).
- **Cryptographic irreversibility** (no BFT certificate, no threshold signatures on blocks).

**They only mean:** more blocks have been **committed on top** on **this node** since **B**, which **may** correlate with **higher effort for a counterparty** to build a **different** retained branch that **this node** would **switch** to — **if** that branch is **received**, **retained**, **valid**, **preferred** by selection, and **within** local reorg policy. **No** probability or cost bound is stated here.

---

## 4. Operational guidance (defaults **TBD** at implementation)

| Parameter | Role |
|-----------|------|
| **`SAFE_CONFIRMATIONS` (integrator-chosen)** | Wait **N** confirmations before **irreversible** off-chain actions (exchange crediting, etc.). |
| **Align with `MAX_REORG_DEPTH`** | If integrators assume automated reorgs up to **d** blocks, they should understand [`reorg_model.md`](reorg_model.md) §4.1: a node may **refuse** deeper rewinds and **diverge** from peers with **larger** limits. **No** formula here claims statistical safety. |

---

## 5. Relationship to branch selection and reorg

- **Committed** tip drives **confirmations**.
- **Reorg** can **drop** blocks from the committed prefix; confirmations for those blocks **collapse** (they are no longer **canonical** here).
- **Ingress** filtering ([`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md) §3) can delay **when** a node **learns** blocks; confirmations are **not** comparable across nodes **without** context.

---

## 6. Peer defense vs confirmations

Firewall or **disconnect** does **not** strengthen **chain** guarantees. It only changes **which data** the node **sees**.

---

## 7. Compatibility with V1 / V2

Linear **single-tip** deployments: confirmations are **depth below tip**; V2 reference had **no** multi-branch reorg. V3 adds **reorg** and **local** reorg limits — integrators must **not** assume **linear forever**.

---

## 8. Open questions

1. **CLI** to print committed tip, retained tips, and **effective** `MAX_REORG_DEPTH` (operator aid).
2. **Checkpoint sync** later: define confirmations when history is **partial** (likely **N/A** until backfill complete).
