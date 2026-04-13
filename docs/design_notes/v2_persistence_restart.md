# V2 design note: persistence and restart semantics

Short operational contract for **node hardening** (V2). Does **not** change V1 transaction/block validity. Full scope: [`v2_scope.md`](../v2_scope.md). Project decisions: [`v2_scope.md`](../v2_scope.md#project-decisions-v2).

---

## `chain.blocks`

- **Recovery:** On detectable corruption, truncation, or decode failure, the node **fails closed** (refuse to load the chain / refuse to advance committed state from that path), except for the narrow tail case below.
- **Narrow tail cleanup:** If `chain.blocks` has at least one complete block frame and then ends with only 1-3 extra bytes (an incomplete next length prefix), startup truncates those bytes, returns `repaired = true`, and the reference CLI logs a `[storage]` repair line. This does **not** truncate a complete or partially described frame; it preserves replay from the last complete frame.
- **Operator action:** Beyond the narrow tail cleanup above, recovery is **manual**: restore from backup, replace the file, or follow an **explicit** documented procedure (e.g. delete `chain.blocks` and re-sync from peers from genesis—operator accepts re-download cost). Any future tool that mutates chain data must be **opt-in** and documented, not implicit.
- **V2 implementation:** Prefer clear errors and exit codes over partial startup with an undefined tip.
- **On-disk layout (local only; not a wire protocol change):**
  - **New / empty files:** After an 8-byte magic header (`TRILBC01`), each record is `u32_be len` + `encode_block` payload + `u32_be CRC-32` (IEEE) over that payload. Load verifies CRC; mismatch or truncation → fail closed.
  - **Legacy files** (no magic): Unchanged format `u32_be len` + payload per record. Still loaded and appended in that format; there is **no automatic in-place migration** to the V2 layout.
- **Append path:** Each new record is written in one `write_all` (plus `sync_all`) where practical, after the one-time magic write on first append for new files.
- **Append/sync errors (running process):** If a `chain.blocks` append or `sync_all` fails after any write may have reached the file, the in-process block store is treated as **poisoned** for the remainder of that run: no further appends, local sealing exits **fail-closed**, and the TCP handler refuses peer work that would extend the chain. **Restart only after** repairing or restoring `chain.blocks` (same rule as uncertain on-disk state).

---

## Local sealing (reference `run` loop)

- Transactions removed from the mempool **only after** the sealed block is **durably** appended to `chain.blocks`. If persistence fails, the node rolls back the in-memory tip; the mempool stays equivalent to the pre-seal FIFO (no re-queue-to-back reordering).

---

## `pending_tx.tril`

- **Intent:** The file is a **durable queue** of user-submitted transactions before they enter the in-memory mempool.
- **Cross-process lock:** `.pending_tx.lock` in the same directory is an **advisory exclusive lock** held for the full **append** (`send`: open → write frame → sync) and full **drain** (`run`: read → parse → mempool admit → atomic rewrite). This prevents a concurrent `send` from being dropped when a drain replaces the file, and prevents interleaved frame writes from concurrent `send` processes. Lock acquisition failure is **fail-closed** (explicit error).
- **Enqueue (`send`):** Each queued transaction is written as one length-prefixed frame in a **single** append write (+ sync), so another process does not observe a lone length prefix without the body from that enqueue.
- **Atomic replace:** The implementation writes a temp file, **fsync**’s it, **rename**’s over `pending_tx.tril`, then (on Unix) attempts **fsync** on the **parent directory** so the rename is less likely to disappear after a crash; directory fsync failure is logged as a warning, not a silent success.
- **No silent loss:** After V2 hardening, draining the file must **not** clear it until submitted txs are accounted for: if any frame cannot be accepted into the mempool, the implementation must **either** leave the file unchanged **or** rewrite it to contain **only** not-yet-accepted frames **in order** (same relative order as before). **Do not** clear the file while txs were parsed but not successfully queued.
- **Rewrite failure:** If rewriting the file fails after some frames were admitted into the mempool, the implementation must **restore the prior mempool FIFO** (or otherwise abort without leaving RAM and file inconsistent).
- **Duplicates:** Mempool deduplication by `tx_hash` prevents **double application** of the same logical tx; logging should make rejects visible.
- **Parse errors:** Garbled or truncated `pending_tx.tril` → **fail the drain** for that cycle, **do not** partially clear; operator fixes or deletes the file under guidance.

---

## Genesis and data directory

- **Binding (V2):** If the node writes **data-dir metadata** (e.g. expected genesis `state_commitment_hex`), then on subsequent startup a **mismatch** with the loaded `genesis.toml` is a **hard refusal** (exit; do not run with mixed genesis and stored chain).
- **First run:** If no binding exists, the reference node creates it on the first successful **`run` or `send`** (not during `init`), after loading `genesis.toml`. The path is serialized with **`.genesis_bind.lock`**. Content is written to a unique temp file, then installed with **`hard_link`** into `genesis_bind.toml` so an existing binding is **not** silently overwritten: if the final name already exists, the implementation re-reads and verifies. If `hard_link` cannot be used, **`rename`** is used only while holding the bind lock (still no concurrent installer). Concurrent first runs with the **same** genesis converge; with **different** genesis, the loser fails verify after the winner installs. **Re-binding is not a casual operator action:** removing `genesis_bind.toml` after changing `genesis.toml` is **only** appropriate when **`chain.blocks` has no committed history yet**; if history exists, **do not** repoint a populated data directory at another genesis—use a **fresh data directory** or an **intentional reset** of `chain.blocks` and re-sync.

---

## Startup vs peer vs log-only

| Situation | Action |
|-----------|--------|
| Local genesis commitment disagrees with data-dir binding | **Refuse startup** |
| `chain.blocks` corrupt / unreadable | **Refuse startup** (fail closed) |
| `pending_tx.tril` unreadable | **Refuse drain** for that cycle; may still run if rest of node policy allows; prefer **visible error** |
| Peer advertises different genesis (when handshake exists) | **Disconnect peer** (does not change block validity rules) |
| Peer-reported height / tip for sync | **Advisory only**; see [`v2_scope.md`](../v2_scope.md#sync-invariant-v2) |

---

## On-disk artifacts (reference node)

This section is **node implementation** and **operator** documentation. It is **not** a protocol rule: honest nodes must still agree on block/tx validity per V1; how each implementation durably stores history is a local concern.

| File | Role | Durable? |
|------|------|----------|
| `genesis.toml` | Source document for height-0 allocations (read at startup / `send`). | Operator-maintained |
| `genesis_bind.toml` | Binds the data directory to one genesis **state commitment** hex. Created on first successful `run` or `send` after genesis load. | Yes, once written |
| `.genesis_bind.lock` | Serializes bind install (advisory). | Lock file only |
| `chain.blocks` | Append-only stored blocks **after** genesis: **V2** = magic `TRILBC01` + repeated `len + encode_block + CRC-32(payload)`; **legacy** = repeated `len + encode_block` only. Genesis block is **not** stored; replay = `from_genesis` + apply each decoded block in order. | Yes, after successful `sync_all` on append |
| `pending_tx.tril` | Durable queue before mempool (`send` append; `run` drain + atomic replace). | Yes, per frame after append sync |
| `.pending_tx.lock` | Serializes pending file access (advisory). | Lock file only |
| `wallet.seed` | Signing key material. | Operator secret |

### What “fail-closed” means at startup

- **`chain.blocks`:** Any malformed frame, CRC mismatch, full length prefix with missing body/CRC, decode error, or replay failure (`load_blockchain_from_disk`) → **startup refuses** to build a chain tip (clear error; no partial “best effort” tip). A 1-3 byte incomplete next length-prefix tail after a complete frame is truncated and logged as the only automatic repair.
- **`genesis_bind.toml` vs `genesis.toml`:** Mismatch → **startup refuses** (do not mix genesis and an existing bound directory). Deleting `genesis_bind.toml` alone to “fix” mismatch is **unsafe** once `chain.blocks` has history.
- **`pending_tx.tril`:** Garbled file → **`run` refuses that drain cycle** with an error; the file is **not** truncated. The process may continue (reference `main` logs and retries next interval); operator must fix or delete the file per guidance below.

### Restart and replay equivalence

- **Cold start:** Missing `chain.blocks` → height 0, genesis-only state (same as empty file).
- **After normal shutdown:** Reloading `chain.blocks` with the **same** `genesis.toml` replays blocks in order; committed balances/nonce/tip hash **must** match what a continuous run would have after the same block sequence (deterministic `append_block`).
- **Idempotent read:** Reading `chain.blocks` multiple times yields the same logical chain (pure function of file bytes + genesis).
- **`pending_tx.tril` after restart:** Frames not yet drained remain on disk; next `run` drain attempts admission again (ordering preserved; no silent clear on parse failure).

### In-process-only: `BlockStore` poison flag

- If an append hits **I/O or sync failure**, the in-memory `BlockStore` sets **`poisoned`** and refuses further appends for **that process**.
- The poison flag is **not** persisted in `chain.blocks`. A **new** `BlockStore` after restart does **not** start poisoned.
- **However:** if the crash left `chain.blocks` **partially written** beyond the narrow 1-3 byte incomplete length-prefix tail, the **next** startup fails at **load** (fail-closed), not at poison. Operator must restore a consistent file.
- Integration tests for reload live under `node/tests/restart_matrix_v2.rs` and `node/tests/persistence_v2.rs`; poison reopen behavior is covered in `storage` unit tests.

### Operator recovery (concise)

| Symptom | Action |
|---------|--------|
| Startup error loading `chain.blocks` | Restore from backup, delete file and re-sync from peers (replays from genesis), or replace with a known-good copy. |
| Binding mismatch | Prefer aligning `genesis.toml` with `genesis_bind.toml`. **No committed history yet:** removing `genesis_bind.toml` can allow re-bind after a genesis change. **`chain.blocks` already has history:** do **not** casually re-bind—use a **fresh data dir** or **reset/re-sync** the chain (replace/remove `chain.blocks` deliberately). |
| `pending_tx.tril` parse / drain error | Fix or remove the file after inspection; do not assume partial auto-recovery beyond “file left unchanged on parse failure”. |
| `run` exited after persist error | Treat `chain.blocks` as **suspect**; inspect length/decodability before restarting production. |

### Not guaranteed yet (backlog / honesty)

- **Automatic repair** of a torn last frame after a complete length prefix, body, or CRC byte has appeared (explicitly out of scope; fail-closed instead). Only the 1-3 byte incomplete length-prefix tail is repaired.
- **On-disk mempool** (in-memory only except via `pending_tx.tril`).
- **Multi-node simultaneous crash** scenarios beyond what CLI E2E and library tests cover.
- **`pending_tx.tril`:** Stronger durability than the documented frame + lock + replace behavior is **not** implied; operators should not assume extra on-disk guarantees.

---

## Auto-repair

**Default: fail closed.** The only automatic `chain.blocks` mutation is truncating a logged 1-3 byte incomplete next length-prefix tail after a complete frame. No automatic truncate-to-tip, full-frame repair, CRC repair, replay repair, or genesis-backed state rewrite is performed without explicit operator action or a dedicated, documented tool.
