# V3-08a — Deterministic `BlockIndex` rebuild (design note)

## A. Purpose

This document is a **V3-08a design and specification note**: it defines how a **future** live V3-capable node could **deterministically rebuild** an in-memory [`BlockIndex`](../node/src/v3/block_index.rs) from **persisted** canonical chain data (and, much later, from any **approved** side-branch store), **without** changing runtime behavior today.

**This is not live integration.** No fork-choice activation, no reorg execution, no new storage formats, no CLI or network changes, and no wiring of `crate::v3` into `append_block`, `try_append_network_block`, or ingest paths.

**Reads with:** [`v3_scope.md`](v3_scope.md), [`v3_integration_readiness_audit.md`](v3_integration_readiness_audit.md), [`reorg_model.md`](reorg_model.md) (especially §9), [`fork_choice.md`](fork_choice.md), [`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md), [`v3_test_plan.md`](v3_test_plan.md).

---

## B. Why this matters

Future **height-first branch selection** ([`fork_choice.md`](fork_choice.md)) and **reorg planning** ([`reorg_plan.rs`](../node/src/v3/reorg_plan.rs), [`reorg_preflight.rs`](../node/src/v3/reorg_preflight.rs)) assume a **trustworthy DAG-shaped view** of retained blocks: `block_hash` → `(parent_hash, height)` with **hardened** walks toward a single height-0 root ([`path_from_tip_toward_genesis`](../node/src/v3/block_index.rs), [`validate_block_ancestry`](../node/src/v3/block_index.rs)).

That view must be **reproducible from disk** after restart, partial sync, or recovery. If rebuild is ambiguous, non-deterministic, or silent on corruption, the node could plan reorgs from a **wrong topology**, fail open on torn frames, or disagree with **cold replay** of `chain.blocks` ([`load_blockchain_from_disk`](../node/src/storage.rs)).

---

## C. Current state (`node/src/v3/block_index.rs`)

| Topic | Today |
|--------|--------|
| **Data shape** | In-memory `HashMap`: `block_hash` → [`BlockIndexEntry`](../node/src/v3/block_index.rs) `{ parent_hash, height }`. |
| **Walks / forks** | [`path_from_tip_toward_genesis`](../node/src/v3/block_index.rs): cycle-safe, missing-link and **height-consistent** parent checks. [`fork_slices_between_tips`](../node/src/v3/block_index.rs): common ancestor + **tip-first** suffixes for reorg planning. |
| **Ancestry** | [`validate_block_ancestry`](../node/src/v3/block_index.rs): walk to a height-**0** row whose `parent_hash` is **empty** (index “root” convention used in tests). |
| **Tests** | Broad unit coverage in isolation (fork shapes, cycles, malformed height, duplicates, etc.). |
| **Not done** | No loader from [`BlockStore`](../node/src/storage.rs) / `chain.blocks`. No coupling to [`Blockchain`](../node/src/blockchain.rs) tip metadata. **Not** authoritative for live consensus today. |

**Deprecated helper:** [`ancestors_to_height_zero`](../node/src/v3/block_index.rs) is explicitly **not** for reorg logic; rebuild and integration code must use hardened paths only.

---

## D. Required future inputs

A future `rebuild_block_index_from_storage(...)` (name illustrative) would need:

1. **Canonical ordered blocks**  
   Decoded [`Block`](../node/src/block.rs) values in **on-disk append order** (the order returned by [`BlockStore::read_all_blocks`](../node/src/storage.rs) / [`read_all_blocks_repairing_tail`](../node/src/storage.rs)), representing the **committed** linear chain **excluding** the genesis row from the file (see §E).

2. **Block identity**  
   Each non-genesis block’s `block_hash` string must match [`Block::compute_block_hash`](../node/src/block.rs) / [`validate_block_hash`](../node/src/block.rs) under the same encoding the node uses everywhere (hex or digest string format — today one canonical string per block).

3. **Parent linkage and height**  
   From each `Block`: `previous_hash` → parent `block_hash`; `height` → `BlockIndexEntry.height`. These must be **consistent** with each other and with the synthetic genesis row (§E).

4. **Genesis binding**  
   The operator-selected [`Genesis`](../node/src/genesis.rs) document (and thus [`Blockchain::from_genesis`](../node/src/blockchain.rs)) fixes **state** at height 0. The **logical** genesis **block** is [`Block::genesis`](../node/src/block.rs) (not stored in `chain.blocks`; see [`storage.rs`](../node/src/storage.rs) module comment). Rebuild must **derive** the expected genesis `block_hash` and reject a chain whose first stored block does not extend that hash at height 1.

5. **Structural validation policy**  
   At minimum, the same **structural** checks needed to trust the index for planning: `basic_validate` on each decoded block (hash preimage, tx dedupe, etc.), monotonic **height** along the canonical list, and **parent_hash** matching the previous block’s `block_hash`.  
   **Full state replay** (transactions) is required for **state / tip honesty** but is a **separate** gate from “index topology is internally consistent”; an implementation may choose to fail-closed on both in one pass or split **topology rebuild** vs **state replay** with explicit ordering — if split, topology must still **fail closed** when linkage fails.

6. **Optional: ingress-only checks**  
   [`validate_block_vs_local_time`](../node/src/consensus.rs) is **not** deterministic from bytes alone. Cold rebuild should **not** treat wall-clock rejection as rewriting history; align with [`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md) §2 vs §3.

7. **Future side-branch bytes**  
   When [`reorg_model.md`](reorg_model.md) §9 side-branch retention exists, additional inputs (hash-keyed store, caps, eviction) feed the same `BlockIndex::insert` **after** formats and trust rules are specified. **V3-08a does not define side-branch formats.**

8. **Corruption / repair signaling**  
   Today [`read_all_blocks_repairing_tail`](../node/src/storage.rs) can truncate a **short** torn tail and return `repaired = true`. A rebuild API should surface that **explicitly** (operator-visible) and still require **deterministic** subsequent validation.

---

## E. Deterministic rebuild algorithm (high level)

**Precondition:** Caller supplies `genesis: &Genesis` (same binding as [`load_blockchain_from_disk`](../node/src/storage.rs)) and a path (or in-memory byte slice) to `chain.blocks`.

1. **Load persisted chain data**  
   Use the same framing rules as today: optional V2 magic `TRILBC01`, then legacy or V2 length-prefixed frames with optional CRC ([`storage.rs`](../node/src/storage.rs)). Prefer the same entry point as production (`read_all_blocks_repairing_tail` vs strict `read_all_blocks`) as a **documented** policy choice; either way, **do not** silently ignore CRC mismatch or truncated mid-frame bodies.

2. **Verify genesis binding**  
   Instantiate the expected genesis block hash from [`Block::genesis`](../node/src/block.rs) (constant `GENESIS_HASH` in the reference implementation) and ensure it matches what [`Blockchain::from_genesis`](../node/src/blockchain.rs) would use as the parent of height **1**. Reject **GenesisMismatch** if the configured `Genesis` cannot be loaded or conflicts with hard-coded genesis block invariants.

3. **Decode blocks**  
   For each frame, `decode_block` → `Block`. Any decode error → **DecodeFailure** (fail closed).

4. **Structural validation per block**  
   - `basic_validate` (includes `validate_block_hash`).  
   - For the **canonical linear** rebuild: enforce **strict sequential heights** starting at **1** for the first stored block, with `previous_hash` equal to the **prior** block’s `block_hash` (after the synthetic index row for genesis is inserted — step 5).  
   - Parent-interval timestamps ([`validate_block_timestamps_vs_parent`](../node/src/consensus.rs)) require the **parent `Block`** payload; maintain a `prev_block` cursor while iterating.

5. **Insert into `BlockIndex` in deterministic order**  
   - Create empty [`BlockIndex`](../node/src/v3/block_index.rs).  
   - **Synthesize the root row** for the genesis **hash** used on the wire (`Block::genesis().block_hash`, i.e. `GENESIS_HASH`):  
     `BlockIndexEntry { parent_hash: String::new(), height: 0 }`.  
     This matches [`validate_block_ancestry`](../node/src/v3/block_index.rs)’s requirement that the height-0 root use an **empty** `parent_hash`, even though the in-memory [`Block`](../node/src/block.rs) genesis uses sentinel `previous_hash == "GENESIS"` (that field is **not** copied verbatim into the index root row).  
   - For each stored block in file order: `insert(block.block_hash.clone(), BlockIndexEntry { parent_hash: block.previous_hash.clone(), height: block.height })`.  
   - If `insert` replaces an existing entry → **DuplicateBlockHash**.

6. **Ancestry consistency**  
   After insertion, call [`validate_block_ancestry`](../node/src/v3/block_index.rs) on the **canonical tip** hash. If one or more stored blocks were inserted, that tip is the **last** stored block’s `block_hash`. If **no** stored blocks exist (missing `chain.blocks`, empty file, or zero decodable frames — matching V2 **genesis-only** load), the index contains **only** the synthetic genesis row from step 5; the canonical tip is then **`Block::genesis().block_hash`** (`GENESIS_HASH`) with `parent_hash == ""` and `height == 0`, and ancestry validation runs **on that hash** (a single-row valid chain). Optionally validate intermediate prefixes in **O(n)** when `n > 0`; for a linear chain, tip validation plus duplicate-freedom along the scan is sufficient.

7. **Canonical tip candidate**  
   **Non-empty disk chain:** for **today’s** single canonical file, the tip is the **last** valid block’s `block_hash` and `height`; **file order + linkage** define the committed chain until metadata exists ([`reorg_model.md`](reorg_model.md) §9.3). **Empty disk chain:** same as step 6 — tip is the synthetic genesis root only; this aligns with [`load_blockchain_from_disk`](../node/src/storage.rs) when the file is missing or yields no frames (genesis-only [`Blockchain`](../node/src/blockchain.rs)).

8. **Side branches (future)**  
   Do **not** merge side-branch hashes until **retention format, eviction, and trust** are specified (V3-08c track). If a future reader encounters an unknown side-branch artifact version → **UnsupportedSideBranchFormat**.

9. **Fail closed**  
   On any of: missing genesis row synthesis, **when at least one stored block is present:** first block height ≠ 1 or parent ≠ genesis hash, height gaps, `previous_hash` mismatch vs prior tip, duplicate hash, `basic_validate` / hash failure, CRC / decode errors, or ancestry walk failure → **abort rebuild** with a typed error; leave live chain untouched (this doc does not prescribe whether partial `BlockIndex` is visible — default **no**). A **genesis-only** result (step 6–7) is **success**, not an error.

---

## F. Failure model (future typed errors)

Implementations should expose **structured** failures suitable for logs and tests. Names below are **spec-level**; Rust enums may differ but should preserve these distinctions.

| Category | When |
|----------|------|
| **MissingGenesis** | No `Genesis` / config binding available to anchor rebuild. |
| **GenesisMismatch** | Stored chain does not extend the expected genesis `block_hash`, or genesis document vs hard-coded genesis block rules disagree. |
| **DecodeFailure** | Frame decode (`decode_block`) or framing parse failed. |
| **DuplicateBlockHash** | Same `block_hash` would be inserted twice. |
| **MissingParent** | Linkage references a parent hash not present in the index (should not occur on linear scan if enforced incrementally — kept for hash-store–first futures). |
| **HeightMismatch** | `block.height != parent.height + 1` or first stored block height ≠ 1. |
| **ParentHashMismatch** | `previous_hash` does not match the expected parent’s `block_hash` for the canonical sequence. |
| **InvalidBlockHash** | `validate_block_hash` / `basic_validate` failure. |
| **InvalidAncestry** | [`validate_block_ancestry`](../node/src/v3/block_index.rs) or [`BlockPathError`](../node/src/v3/block_index.rs) after insert (cycles, malformed root, etc.). |
| **StorageCorruption** | CRC mismatch, truncated mid-frame, ambiguous tail per V2 rules, or I/O errors reading bytes. |
| **UnsupportedSideBranchFormat** | Reserved for future side-branch blobs not recognized by this node version. |

**Note:** [`BlockPathError`](../node/src/v3/block_index.rs) already models many topology failures for **in-memory** use; a rebuild layer may **wrap** or **map** those into the table above for a single operator-facing type.

---

## G. Relationship to storage

- **V2 `chain.blocks`** is an **append-only linear log** of encoded blocks ([`reorg_model.md`](reorg_model.md) §9.6). Genesis is **not** a frame; replay today uses [`load_blockchain_from_disk`](../node/src/storage.rs) = read frames + [`Blockchain::from_genesis`](../node/src/blockchain.rs) + sequential [`append_block`](../node/src/blockchain.rs).
- **Rebuild for `BlockIndex`** should use the **same bytes and decode path** as that replay for **canonical** rows, so the index and full-chain replay **agree** on ordering and membership.
- **Side-branch hash store** (§9.1–9.2) is **not** implemented; until it is, rebuild from **only** `chain.blocks` cannot populate competing tips—only the **single** linear history the file represents.

---

## H. Relationship to `branch_select`

[`branch_select`](../node/src/v3/branch_select.rs) is **pure**: given [`TipDescriptor`](../node/src/v3/branch_select.rs)s, it orders by height then TB1 hash. It does **not** check that tips exist in a `BlockIndex` or that they are valid blocks.

**Rule:** Candidate tips fed to `select_preferred_tip` must come from a **validated** index (or equivalent) so that “higher height wins” is not applied to **random** hashes. Rebuild is the **startup** path to trust that set for the **canonical** branch; side branches await §9.

---

## I. Relationship to `replay_sandbox`

[`ReplaySandbox`](../node/src/v3/replay_sandbox.rs) assumes **honest, complete** `blocks_by_hash` and parent headers for every replayed block. A correct **`BlockIndex`** is **necessary but not sufficient**: the sandbox still needs **full block bodies** and parent maps for execution.

Deterministic rebuild from storage is one **building block** to produce **consistent** `block_hash` → header facts and to enumerate suffixes for planning; the sandbox remains **inert** and **separate** from this note.

---

## J. Readiness checklist (V3-08a)

| # | Criterion | Status (this document) |
|---|-----------|-------------------------|
| 1 | Documented rebuild **inputs** | §D |
| 2 | Documented **deterministic** rebuild **algorithm** | §E |
| 3 | Documented **failure** cases | §F |
| 4 | Documented **storage** assumptions (V2 frames, genesis not on disk) | §G, §D |
| 5 | Documented **side-branch** limitation | §D.7, §E.8 |
| 6 | Documented relation to **branch selection** | §H |
| 7 | Documented relation to **replay sandbox** | §I |
| 8 | **No live wiring** added | Satisfied by process; code unchanged in this milestone |

---

## K. Non-goals (explicit)

- No **live fork-choice** activation or calls from network/mempool paths.
- No **reorg execution** or state mutation beyond what existing V2 code already does.
- No **side-branch storage** implementation or new on-disk products.
- No **storage migration** or wire-format change.
- No **network**, **mempool**, or **CLI** behavior changes.
- No **production finality** or partition-safety claims beyond existing docs.

---

## Conclusion

V3-08a specifies a **deterministic**, **fail-closed** path from **today’s** `chain.blocks` bytes → decoded [`Block`](../node/src/block.rs)s → **`BlockIndex` rows**, including the **critical normalization** of the genesis **root row** (empty `parent_hash` at height 0 in the index vs genesis **block** sentinels in `Block`). Side-branch indexing and hash-first stores remain **out of scope** until later milestones.
