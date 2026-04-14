# Trilogicon V3 — Reorg model

**Status:** design for **future** implementation. **No** reorg execution in the reference node in this docs-only phase. **Does not** change `append_block` semantics until implementation is approved.

**Reads with:** [`fork_choice.md`](fork_choice.md), [`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md), [`finality.md`](finality.md), [`v3_test_plan.md`](v3_test_plan.md).

---

## 1. Purpose

When **height-first branch selection** ([`fork_choice.md`](fork_choice.md)) picks a tip that is **not** an extension of the **current committed tip**, the node performs a **reorg**: undo committed state along the **old suffix** and **apply** the **new suffix** from the **fork point**.

This document defines **boundaries**, **`MAX_REORG_DEPTH` classification**, **storage design requirements**, **mempool and old-suffix transaction behavior**, and **assumptions**.

---

## 2. Terminology

- **Committed tip:** last block applied to **canonical** state and recorded in **canonical tip metadata** (§9.3).
- **Fork point:** deepest block **common** to old and new canonical prefixes.
- **Old suffix:** blocks on the **previous** canonical chain **after** the fork point.
- **New suffix:** blocks on the **new** canonical chain after the fork point through the new tip.

---

## 3. Assumptions

1. **Validated inputs:** New suffix blocks passed **deterministic chain validity** ([`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md) §2) for their respective parents **before** reorg execution.
2. **Deterministic application:** Rollback + apply matches **replay** of genesis through the new canonical chain.
3. **Single committed tip** at rest: reorg completes **atomically** from an external observer’s perspective (§9.4).

---

## 4. Reorg boundaries

### 4.1 `MAX_REORG_DEPTH` — classification (**resolved**)

**`MAX_REORG_DEPTH` is a local fail-closed halt policy** on **automated reorg execution**.

| Aspect | Classification |
|--------|----------------|
| **What it limits** | Length of the **old canonical suffix** (blocks to unwind) in one **automated** reorg. |
| **Consensus constant?** | **No.** It is **not** a protocol-era constant all peers must share for **validity** or **branch selection**. |
| **Fork choice** | **Unchanged.** The **abstract** preferred tip from [`fork_choice.md`](fork_choice.md) may still be computed; **this node** may **refuse** to **execute** the reorg if the unwind exceeds the **local** bound. |
| **Behavior when exceeded** | **Fail-closed:** **do not** perform the automated reorg; **remain** on the **current committed tip**; **surface** explicit operator-visible reason (log / `operator_msg` / advisory state). **Manual** recovery (resync, override flag — **future tooling**) is out of scope here. |
| **Configuration** | Implementation chooses **CLI / config / compile-time default**; **must** document default and semantics in release notes. |

**Rationale:** bounds worst-case work and disk churn **per node** without pretending **global** agreement on depth.

**Interoperability note:** two nodes with different `MAX_REORG_DEPTH` may **both** be **correct** relative to this spec while **committed tips differ** after seeing the same blocks. Operators align **policy** to their **risk** model; see [`finality.md`](finality.md).

### 4.2 Genesis

**Never** reorg **before** or **across** genesis.

### 4.3 Same-tip no-op

If the selected tip **equals** the committed tip, **no** reorg.

---

## 5. Execution model (high level)

1. Compute fork point, old suffix, new suffix; check §4.1.
2. Exclusive lock on chain state + stores (§9).
3. Roll back state along old suffix (reverse apply or reload — implementation).
4. Apply new suffix in height order.
5. Persist **canonical tip metadata** and canonical chain sequence per §9.
6. **Mempool pipeline** §7–8.
7. Release locks.

**If step 4 fails:** follow §9.4 / partial-failure rules.

---

## 6. Relationship to branch selection

[`fork_choice.md`](fork_choice.md) yields the **preferred** tip. **Reorg execution** aligns **committed** state with that tip **subject to** §4.1. A node may be **stuck** on a shorter committed chain while **abstractly** preferring a taller retained tip — **document** this for operators.

---

## 7. Mempool after reorg (queued transactions)

**Principle:** mempool is **local** and **non-consensus**; minimum rules prevent **obvious garbage** after state changes.

### 7.1 Mandatory purge (normative)

After a successful reorg, **re-evaluate** every **queued** (not yet committed) transaction against **new** post-reorg state:

1. **Remove** txs that fail **acceptance** rules (stale nonce, insufficient balance, structural invalidity, etc.).
2. **Duplicates:** same **`tx_hash`** — **dedupe** per existing mempool semantics (**one** logical entry).

### 7.2 FIFO among **surviving** queued txs

Among txs **still valid** after §7.1:

- **Default (recommended):** **preserve relative FIFO order** among survivors (no full mempool **reset** unless operator explicitly requests **clear** in a **future** feature).
- **Alternative (documented implementation choice):** full reset after reorg is **allowed** only if **explicitly** documented and tested; not the silent default.

### 7.3 What this section does **not** cover

Txs **inside committed blocks** of the old suffix — see §8.

---

## 8. Old canonical suffix: committed block transactions

Transactions that were **committed** inside blocks on the **old suffix** are **no longer on the canonical chain** after a successful reorg (their effects were rolled back).

### 8.1 Default: no automatic mempool re-injection

**By default**, the node **does not** automatically **re-queue** txs drawn from old-suffix block bodies. They may **reappear** only through **normal ingress** (gossip, `send`, sync) like any other tx bytes.

**Rationale:** avoids surprising **order** changes and duplicate **complexity**; keeps mempool a **forward** queue.

### 8.2 Optional reconciliation (local policy)

An implementation **may** offer **optional** reconciliation: after §7.1, **iterate** txs from old-suffix blocks in **deterministic order** (**height ascending**, then **transaction index within block**) and attempt **try_submit** (or equivalent) against the **new** state.

**Re-admission rules under reconciliation:**

- **Still valid** on new state → **may** enter mempool **at tail** (after §7.1 survivors) to **avoid** disturbing preserved FIFO among prior queue entries — **or** **document** if interleaving uses a different deterministic rule.
- **Invalid** on new state → **drop**; **no** retry until new blocks or new bytes.
- **Already committed** on the **new** canonical chain (same `tx_hash`) → **reject** as **duplicate / replay** per existing rules; **must not** double-apply.

### 8.3 Duplicates

- **`tx_hash`** is the **dedupe** key across mempool and **committed** chain checks.
- Reconciliation **must** respect the same **duplicate** semantics as gossip paths.

### 8.4 Interaction with `pending_tx.tril`

Pending-on-disk queue: **re-validate** against **new** tip on drain; **do not** resurrect entries that fail validation; **head-of-line** semantics remain as V2 unless a **versioned** change says otherwise.

---

## 9. Storage design (**required before implementation**)

This section is a **design requirement**, not an implementation. No Rust API is fixed here.

### 9.1 Block store by hash

- Persist (or cache) blocks **addressable by `block_hash`** (content-defined key).
- Support **lookup** of raw block bytes or decoded `Block` by hash for **replay** and **side-branch** retention.

### 9.2 Parent index

- Maintain **`block_hash → parent_hash`** (or equivalent) for **traversal** from any retained tip back to genesis.
- Enables **fork point** computation and **suffix** enumeration.

### 9.3 Canonical tip metadata

- Persist **committed tip** `block_hash` and **height** (and optionally **total difficulty** if ever added) in **durable** metadata separate from “all blocks ever seen.”
- **Startup** must know **which hash is canonical** without scanning the entire hash store.

### 9.4 Crash recovery

- **Invariant:** after **crash** at any point, the node either loads a **consistent** `(canonical tip metadata, state)` pair or **fails closed** into a **recoverable** mode (operator-directed repair).
- **Forbidden:** silently serving **state** that does not match **replay** from genesis through **recorded** canonical tip.
- **WAL / two-phase** or **write ordering** details are **implementation** choices; **tests** required per [`v3_test_plan.md`](v3_test_plan.md) **ST-**\*).

### 9.5 Replay procedure

- **Cold verification:** ability to compute **state** (or state commitment) by **sequential apply** from genesis following **canonical** chain order **read from storage**.
- Used for **recovery**, **tests**, and **audits**.

### 9.6 Migration from V2 `chain.blocks`

- **V2** file is **append-only linear** canonical log.
- **V3** migration path (**required** in implementation PR): **one-time** or **lazy** import of existing frames into **hash store** + build **parent index** + set **canonical tip metadata** from **current** tip.
- **Rollback strategy** if migration aborts: **document** whether V2 file remains **source of truth** until migration **commits** a **marker** (e.g. `migrated_v3` sidecar).

---

## 10. Compatibility with V1 / V2

- **Pure extension** of the committed path: **append-only** growth without competing branches remains **valid** V3 behavior.
- **Ingress** and **defense** remain V2-class until wire era changes.

---

## 11. Open questions (pre-implementation review)

1. **Migration:** single **offline** migration tool vs **inline** on first **V3** start.
2. **Side-branch retention cap** before **prune** (affects which tips exist for selection).
3. **Pause serving** blocks during reorg (recommended default **yes** — confirm in network doc).
4. **Operator override** to raise `MAX_REORG_DEPTH` for one-shot sync (security tradeoff — **future**).
