# Trilogicon V3 — Protocol chain rules (design)

**Status:** design document. **Does not** change runtime behavior until implementation is approved and merged with tests per [`v3_test_plan.md`](v3_test_plan.md).

**Reads with:** [`v3_scope.md`](v3_scope.md), [`fork_choice.md`](fork_choice.md), [`reorg_model.md`](reorg_model.md), [`finality.md`](finality.md).

---

## 1. Role of this document

This document fixes **vocabulary** and **layer boundaries** for the V3 **chain layer**: deterministic **chain validity** vs **local ingress/admission**, branch selection, reorgs, finality, and peer defense — without smuggling fork choice into validity or vice versa.

---

## 2. Deterministic chain validity (protocol-only inputs)

**Definition:** A block **B** is **valid** relative to **parent header** **P** and **parent state** **S** when **all** of the following hold. These predicates use **only** `B`’s encoded fields, **P**, **S**, and **protocol-versioned** rules. They **do not** read **local wall clock**, **peer identity**, or **connection order**.

**Included (illustrative; authoritative detail remains V1/V2 specs unless versioned):**

- **Structural / encoding:** magic, fields, lengths, `block_hash` preimage consistency (V1 rules).
- **Linkage:** `B.previous_hash == P.block_hash`, `B.height == P.height + 1` (genesis exception as today).
- **Parent-relative consensus time rules** that are **pure functions of** `(B, P)` — e.g. minimum timestamp gap vs **parent** `timestamp_unix` where the protocol defines them for the active **protocol era**.
- **Transactions:** each tx passes **basic_validate** and **state transition** rules when applied **in order** to the running state from **S**, producing **S'**.

**Excluded from deterministic validity (must live elsewhere):**

- Any check of the form **`B.timestamp_unix` vs local `now`** or **“too far in the future relative to this node’s clock.”** That is **local ingress / admission policy** (§3), **unless** a future **protocol version** elevates a specific numeric rule into **normative chain validity** for all nodes (would require explicit versioning and interop docs).

**Determinism:** For a fixed protocol version, two implementations that evaluate `valid(B, S, P)` on the same inputs **agree** on the result.

**V3 clarification:** Validity does **not** ask “which fork we prefer.” It only asks “**if** **P**/**S** were the parent on **some** branch, is **B** an acceptable next block?”

**This docs phase:** **`append_block` and current network paths remain as implemented** until code changes; this split guides **refactoring** so **deterministic validity** functions are **callable without** wall clock, and **ingress** applies **additional** filters **before** or **after** parsing as designed.

---

## 3. Local ingress / admission policy (non-deterministic across nodes)

**Definition:** Rules that decide whether **this node** **accepts**, **buffers**, or **rejects** a block **at receipt time**, using **local** resources or **local** time.

**Examples (V2-era reference behavior; not exhaustive):**

- Reject or defer a structurally valid block because **`B.timestamp_unix`** exceeds **local `now` + configured future drift** (see [`v2_scope.md`](v2_scope.md) deployment guidance).
- Drop frames under **rate / size** caps before full validation completes.

**Properties:**

- Ingress **may** cause two honest nodes to **temporarily** differ on **which blocks they have validated or retained**, even though **valid(B, S, P)** is the same for both **when** evaluated.
- A block **rejected at ingress** is **not** automatically “invalid forever”; re-offered later (e.g. after clock catches up) may be **admitted** and validated.
- **Documentation** must not label ingress-only outcomes as **deterministic chain invalidity** unless the protocol **version** explicitly merges that rule into §2.

---

## 4. Height-first branch selection (specified elsewhere)

Branch selection answers: among **valid** competing tips the node **retains**, which tip is **preferred** for **canonical** commit. It is **not** `valid(B, S, P)`.

- **Defined only** in [`fork_choice.md`](fork_choice.md).
- Inputs: **validated** blocks only; **no** peer metadata in the selection function (see test plan **PM-**\*).
- **Not** a security or producer-cost model; see [`fork_choice.md`](fork_choice.md) §1.

---

## 5. Reorg execution (specified elsewhere)

Defined in [`reorg_model.md`](reorg_model.md): rollback/reapply, **local** `MAX_REORG_DEPTH` halt, storage, mempool, **old-suffix** transactions.

---

## 6. Finality / confirmations (specified elsewhere)

[`finality.md`](finality.md) — integrator-facing only; **no** implied PoW/stake/BFT.

---

## 7. Local peer-defense policy (non-consensus)

Connection limits, frame caps, strikes, disconnects. **Classification:** [`v2_scope.md`](v2_scope.md).

**V3 note:** Defense must **not** implement **canonical** branch selection (e.g. preferring a tip because of peer IP). Schedule **fetch** only; **select** with [`fork_choice.md`](fork_choice.md).

---

## 8. Network and wire compatibility (V1 / V2 / V3)

- **V1/V2 wire** remains baseline until a versioned wire delta.
- **V3** may keep **same block bytes**; internal representation gains hash store + tip metadata ([`reorg_model.md`](reorg_model.md) §9).

---

## 9. Invariants any V3 implementation must preserve

Given a **fixed canonical block sequence** from genesis: deterministic final state ([`protocol_invariants.md`](protocol_invariants.md) §4), balances, nonces, replay rules for **applied** blocks; invalid blocks never **committed** on canonical chain.

---

## 10. Open questions (pre-implementation review)

1. **Refactor order:** Split **ingress** from **pure `valid(B,S,P)`** in code without behavior change first, or bundle with storage scaffold?
2. **Protocol versioning:** If any **ingress** rule is promoted into §2, bump **protocol era** documentation in the same release.
3. **Pruning:** Retained branch window vs disk ([`reorg_model.md`](reorg_model.md)).
