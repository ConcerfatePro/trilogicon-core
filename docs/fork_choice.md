# Trilogicon V3 — Height-first branch selection

**Status:** specification for **future** implementation. **Not** active in the reference node until merged with code and tests per [`v3_test_plan.md`](v3_test_plan.md). **Does not** modify `append_block` or deterministic validity in this docs-only phase.

**Reads with:** [`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md), [`reorg_model.md`](reorg_model.md).

---

## 1. Purpose and non-goals

This document defines a **total preorder** over **valid competing tips** so implementations with the **same retained validated blocks** apply the **same** ordering when **selecting a preferred tip**.

**Explicit non-goals:**

- This is **not** a **security model**. It does **not** assume proof-of-work, stake, or BFT; it does **not** bound adversary cost or guarantee **network-wide** convergence.
- **Height-first branch selection** is **not** “longest-chain security” — there is **no** producer-cost or work accumulation in V3 phase 1. Wording **“longest chain”** is **avoided** here unless a **later** protocol adds a **defined** cost metric.
- The rule is a **deterministic engineering convention** so nodes **agree on ordering** when they share the same view; **sync**, **retention**, and **local reorg policy** determine whether views align.

---

## 2. Definitions

- **Branch:** a sequence of **valid** (per [`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md) §2) blocks from **genesis** forward, each linking to the previous.
- **Tip:** last block of a branch; identified by **`block_hash`** (hex string per existing encoding).
- **Height `H(T)`:** tip’s `height` field (genesis = 0 per existing model).
- **Candidate set:** tips of branches the node **retains** and has **fully validated** through the tip.

---

## 3. Primary rule: height-first preference

Among two tips **A** and **B**:

- If `H(A) > H(B)`, **A** is **strictly preferred**.
- If `H(B) > H(A)`, **B** is **strictly preferred**.

**Interpretation:** prefer the branch that extends **farther from genesis** by **block count** (height). This is **not** proof of more “work” or economic backing unless a **future** spec adds such a metric.

**Rationale:** simple, cheap, and easy to explain for a chain **without** a defined producer-cost function in phase 1.

---

## 4. Equal height: temporary deterministic tie-break (NOT a security property)

If `H(A) == H(B)` and `A.block_hash != B.block_hash`, tips are **competing same-height forks**. The spec **requires** a **temporary** deterministic tie-break so implementations do not diverge **arbitrarily**:

**Rule TB1 (temporary fallback) — lexicographic tip hash, greater wins:**

- Compare `A.block_hash` and `B.block_hash` as **ASCII lexicographic** strings (hex as produced by the node).
- The tip with the **lexicographically larger** `block_hash` is **preferred**.

**TB1 properties (must be documented to integrators and implementers):**

- **Grindable:** any party who can **influence** `block_hash` (via any **valid** change to block content under V1 preimage rules) can **bias** tie outcomes. TB1 is **not** fair lottery and **not** a security barrier.
- **Not a security property:** it does **not** prevent attacks, only **breaks symmetry** between otherwise equal-height tips.
- **Temporary:** TB1 is a **placeholder** until a **future** protocol version replaces it with a rule tied to an **explicit** economic or security model (e.g. work, stake, or BFT finality) **if** the project adopts one.

**If hashes equal:** tips are the **same** block (collision infeasible under normal assumptions).

---

## 5. Comparing branches via tips only

Ordering is **fully determined** by **tip comparison** under §3–4: greater height decides; equal height uses TB1. **Common ancestor** is not needed for **ordering** but is required for **reorg path** construction ([`reorg_model.md`](reorg_model.md)).

---

## 6. Pruning and incomplete views

Selection is defined only on the **retained** candidate set. Pruned branches **exit** the set until re-fetched. Two nodes with different retention **may** prefer different tips **from their own sets**; **no** global agreement is implied.

---

## 7. Forbidden inputs to selection

**Do not** use peer address, arrival order, wall-clock, or TCP session id as **inputs** to the **selection function**. Those may affect **which blocks arrive** or **when** validation runs, not **ordering** of tips (tests: [`v3_test_plan.md`](v3_test_plan.md) **PM-**\*).

---

## 8. Compatibility note (V1 / V2)

Single-branch deployments: selection **never** differs from the sole tip.

---

## 9. Open questions (pre-implementation review)

1. **Replacing TB1:** criteria for graduating to work-weighted or external finality (separate protocol era).
2. **Smaller vs larger hash:** TB1 uses **larger** wins as an arbitrary convention; confirm no tooling assumes the opposite.
3. **Operator visibility:** whether to log when TB1 **actually** decided (rare) for debugging.
