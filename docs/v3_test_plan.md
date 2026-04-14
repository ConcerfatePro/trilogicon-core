# Trilogicon V3 — Test plan (pre-implementation gate)

**Status:** **required tests** before **any** V3 branch-selection / reorg / storage code merges to `main`. This document does **not** add tests yet; it defines acceptance criteria.

**Reads with:** [`v3_scope.md`](v3_scope.md), [`fork_choice.md`](fork_choice.md), [`reorg_model.md`](reorg_model.md), [`finality.md`](finality.md), [`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md).

---

## 1. Purpose

Ensure branch selection, reorg execution, mempool behavior, storage, and ingress separation **do not**:

- conflate **deterministic validity** with **local clock** or **peer metadata**,
- treat **TB1** tie-break as a **security** mechanism,
- **silently** corrupt state, tip metadata, or balances.

---

## 2. Tests required before implementation merge

### 2.1 Height-first branch selection (pure logic)

| ID | Requirement |
|----|-------------|
| FC-01 | **Height ordering:** greater **height** strictly **preferred** when tips differ in height. |
| FC-02 | **Equal-height TB1:** lexicographically **larger** `block_hash` wins ([`fork_choice.md`](fork_choice.md) §4). |
| FC-03 | **Single tip:** sole candidate **selected**. |
| FC-04 | **Transitivity** on a small synthetic candidate set. |
| FC-05 | **API hygiene:** selection function **signature** does not accept peer id, socket addr, or wall-clock. |

### 2.2 Tie-break / grinding (**TB1 is grindable — not security**)

| ID | Requirement |
|----|-------------|
| FC-06 | **Grinding demonstration:** construct two **valid** same-height competing tips where **only** grindable fields differ (per V1 preimage rules) such that **TB1** flips **winner** — proves tie-break is **not** collision-resistant security. |
| FC-07 | **Determinism:** identical candidate set → identical **ordered** tips across repeated calls / platforms. |

### 2.3 Peer-metadata isolation

| ID | Requirement |
|----|-------------|
| PM-01 | **Shuffled arrival:** same set of validated tips presented in **different** orders → **same** selected tip. |
| PM-02 | **Labeled peer fields:** if validation pipeline attaches **PeerId** / **addr** metadata, **assert** selection **ignores** them (unit test with **injected** bogus metadata on identical blocks). |
| PM-03 | **Static audit / negative:** grep or **forbid** pattern: branch-selection module importing **network** peer types (enforced by test or `cargo udeps` / module path convention — **implementation** choice). |

### 2.4 Deterministic validity vs ingress

| ID | Requirement |
|----|-------------|
| IV-05 | **`valid(B,S,P)`** (or equivalent) **does not** read **system time** — test with **injected** clock or **pure** function boundary. |
| IV-06 | **Ingress path** may reject block **valid** under §2 due to **future drift** vs **mock** `now` — separate test from IV-05. |

### 2.5 Reorg execution

| ID | Requirement |
|----|-------------|
| RE-01 | **No-op** when tip unchanged. |
| RE-02 | **Depth-1 reorg:** balances/nonces match **replay** golden vector. |
| RE-03 | **Parameterized depth** `> 1` vs replay. |
| RE-04 | **`MAX_REORG_DEPTH`:** when old suffix **exceeds** local bound, **no** committed tip change; **explicit** failure / advisory signal (configurable in test). |
| RE-05 | **Atomicity / partial I/O:** fault injection — **either** pre-reorg **or** post-reorg consistent state + tip metadata ([`reorg_model.md`](reorg_model.md) §9.4). |
| RE-06 | **Genesis anchor.** |

### 2.6 Storage / crash recovery (**canonical metadata**)

| ID | Requirement |
|----|-------------|
| ST-01 | **Restart:** after normal shutdown, **canonical tip** metadata + state match **replay** from genesis through tip. |
| ST-02 | **Crash simulation** mid-write (or post-failure recovery path): node **does not** serve **state** inconsistent with stored canonical tip **or** **fails closed** with documented recovery. |
| ST-03 | **Hash lookup:** block retrievable by **`block_hash`** after insert. |
| ST-04 | **Parent index:** walk from tip to genesis matches **expected** chain. |

### 2.7 Mempool and **old-suffix** transaction behavior

| ID | Requirement |
|----|-------------|
| MP-01 | Post-reorg: **stale nonce** txs **removed**. |
| MP-02 | Post-reorg: **insufficient balance** txs **removed**. |
| MP-03 | **FIFO:** among **surviving** valid queued txs, **relative order** preserved per [`reorg_model.md`](reorg_model.md) §7.2 **default** — **or** documented **reset** path tested separately. |
| MP-04 | **Duplicate** `tx_hash` handling unchanged. |
| MP-05 | **`pending_tx.tril`:** no invalid resurrection after reorg ([`reorg_model.md`](reorg_model.md) §8.4). |
| OS-01 | **Old-suffix tx reintroduction:** tx **only** in rolled-back blocks **re-appears** in mempool **only** via **explicit reconciliation** if enabled; **default path** = **no** auto-queue without ingress ([`reorg_model.md`](reorg_model.md) §8.1–8.2). |
| OS-02 | **Reconciliation order:** if reconciliation enabled, **height asc** then **tx index**; **duplicate** of tx already on **new** chain **rejected**. |
| OS-03 | **Same tx** re-submitted after reorg: **accepted** if valid and **not** duplicate of **committed** chain. |

### 2.8 Multi-node / sync

| ID | Requirement |
|----|-------------|
| MN-01 | **Partition heal** within **same** local `MAX_REORG_DEPTH` — tips converge **when** policy allows (document timeouts). |
| MN-02 | **Lagging catch-up** + optional reorg scenario matches **selection** function. |
| MN-03 | **Malformed wire:** fail closed; **no** selection on **invalid** bytes. |

### 2.9 Invariants (regression)

| ID | Requirement |
|----|-------------|
| IV-01 | **Supply / fee burn** after reorg matches replay. |
| IV-02 | **Nonce** monotonicity on **canonical** chain. |
| IV-03 | **Replay:** same signed tx **not** applied twice on canonical chain. |
| IV-04 | **V1/V2 rejection matrix** unchanged for **unchanged** validity paths. |

### 2.10 Platform and CI

| ID | Requirement |
|----|-------------|
| CI-01 | `cargo fmt`, `clippy -D warnings`, `cargo test` on **ubuntu, windows, macos**. |
| CI-02 | `cargo audit` clean or **documented** exceptions. |

---

## 3. Tests **not** sufficient alone

- Selection unit tests **without** **RE-**\* and **ST-**\*) integration.
- Multi-node tests **without** **PM-**\*) isolation guarantees.

---

## 4. Documentation gate (same PR as code)

- [`reorg_model.md`](reorg_model.md) §4.1, §9, §8 **operator** summary.
- [`finality.md`](finality.md) **non-security** disclaimer.
- **`modules.md`** / architecture: validity vs ingress vs selection vs reorg.

---

## 5. Planning phase (now)

No new tests until implementation; this file is the **contract**.

---

## 6. Open questions

1. **FC-06** fixture strategy: minimal **two-tx** blocks differing **only** in grindable field — may require **test-only** keying or harness blocks.
2. **ST-02** fault injection: platform-specific vs pure **mock** store trait.
3. **Migration** tests for **V2** `chain.blocks` import ([`reorg_model.md`](reorg_model.md) §9.6) — **golden** small file + **idempotent** second run.
