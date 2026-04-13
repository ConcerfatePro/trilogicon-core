# Trilogicon V2 — Scope and planning

**Status:** canonical V2 definition for the reference node. V1 is the completed baseline (`docs/v1_scope.md`, `docs/v1_checkpoint.md`). Checklists below track **release alignment** (what shipped vs backlog), not hypothetical future work only.

---

## Canonical definition

**Trilogicon V2** is a **node-hardening release** for the **existing linear V1 protocol**: it improves **peer and session safety**, **linear sync and catch-up**, **persistence, storage, and restart behavior**, **local mempool hygiene**, and **operator observability**—without changing what honest nodes must accept as valid blocks or transactions. **Storage and persistence are part of operational robustness** (same theme as node reliability, not a separate product pillar). V2 **does not** redesign consensus, add fork choice, alter fee economics, or introduce protocol identity fields (such as chain ID) into signed payloads.

---

## Protocol freeze for V2

V2 work must **not** change any of the following unless the project publishes a **new protocol version** document and scope (explicitly **not** V2):

| Frozen area | Meaning for V2 |
|-------------|----------------|
| **Transaction payload and signature rules** | Canonical unsigned payload, field ordering, hashing, Ed25519 verification, and sender binding stay as in V1. |
| **Block header, body, and hash semantics** | Structure, preimage rules, and `basic_validate` criteria for blocks unchanged. |
| **Fee rule** | **Fee burn** only: fees are not paid to another account in V2. |
| **Nonce and replay rules** | Next-nonce expectation and rejection of stale or skipped nonces for **consensus / block application** unchanged. |
| **Genesis model** | Same genesis block in code + same `genesis.toml` allocation model and state commitment rules. |
| **Chain extension** | **Linear extension only**; no fork choice, no reorg, no longest-chain selection. |
| **Chain ID in signed transactions** | Out of scope; remains a **post–V2** protocol decision if ever adopted. |
| **Economics** | No new issuance rules, fee routing, rewards, or treasury behavior. |
| **Validator / staking model** | No protocol-level validator set or staking mechanics. |

**Implementation rule:** If a change would cause two honest nodes with the same genesis and the same received blocks to **diverge on committed state** depending on a **CLI flag, config file, or “local policy” toggle**, it is **not** V2—unless the flag only affects **non-committing** paths (see [Classification](#classification-local-vs-wire-vs-consensus) and [Project decisions (V2)](#project-decisions-v2)).

---

## Why V2 exists after V1

V1 proves the core ledger: accounts, signed transfers, deterministic validation, blocks, and basic multi-node sync. That foundation is necessary but not sufficient for **day-to-day operation** at small scale: peers can be flaky, disks can be interrupted, catch-up paths can be fragile, and debugging without structured signals is slow. V2 closes the gap between “correct protocol core” and **dependable node software**—still minimal, still auditable, but harder to mis-run and easier to diagnose.

---

## Core goals

- **Peer and session safety** without new consensus predicates.
- **Linear sync and catch-up hardening** (same validity rules; better robustness and resource behavior).
- **Persistence and restart semantics** that are explicit, testable, and safe-by-default.
- **Mempool hygiene** strictly as **local policy** that does not change block validity or producer obligations under V1.
- **Observability** to support the above; logging and CLI ergonomics **follow** correctness decisions, they do not replace them.

---

## Classification: local vs wire vs consensus

Use this table before merging any V2 change. When in doubt, treat the item as **consensus-sensitive** and **out of V2**.

| Example | Category |
|---------|----------|
| Log levels; structured log fields; redaction policy | **Local-only** |
| Peer diagnostics (logging why a message was dropped; connection state for operators) | **Local-only** |
| Connection limits, read/write deadlines, inbound payload caps, per-peer rate limits | **Local-only** (must not reinterpret block bytes) |
| **`--mempool-capacity`** (`run`): max in-memory queued txs | **Local-only** — RAM/DoS hygiene; does not change valid txs/blocks; peers need not match |
| Sync retry / backoff / queue caps for fetching blocks | **Wire-compatible but non-consensus** (delivery strategy only) |
| On-disk storage format metadata / file version stamp (for migration and integrity) | **Wire-compatible but non-consensus** (local I/O contract) |
| Exchanging genesis **state commitment** or hash in handshake for **mismatch detection** | **Wire-compatible but non-consensus** if the outcome is **disconnect** (or advisory-only fields); must not redefine valid blocks; **local** genesis/binding mismatch remains **startup refusal** ([Project decisions](#project-decisions-v2)) |
| **Timestamp acceptance knobs** (e.g. max future drift vs local clock on ingress) | **Consensus-sensitive** — already affect whether a block may append; [Project decisions (V2)](#project-decisions-v2) require **deployment-wide standardization**, not casual per-node values; V2 may clarify docs/warnings only unless a **future protocol version** changes the rule itself |
| **Chain ID in signed transactions** | **Consensus-sensitive / out of V2** |
| **Fork choice / reorg** | **Consensus-sensitive / out of V2** |
| **Fee routing** (pay proposer, pool, etc.) | **Consensus-sensitive / out of V2** |
| **Speculative future-nonce mempool** (holding gap nonces in hope of inclusion) | **Consensus-sensitive / out of V2** if it can change **which** valid txs appear in produced blocks vs a reference V1 node; pure **drop/revalidate** of already-invalid-or-stale txs is [in scope](#in-scope) |
| **Auto repair / silent truncation** of `chain.blocks` or state | **Consensus-sensitive / out of V2** unless specification proves equivalence to replay from genesis; default is **fail closed** and operator-directed recovery |

---

## Project decisions (V2)

These are **repo decisions** for implementation planning. Details and operational wording: [`design_notes/v2_persistence_restart.md`](design_notes/v2_persistence_restart.md).

### Timestamp and drift (`--max-future-drift-secs`)

- The knob remains **consensus-relevant**: it gates whether an **inbound** block may be appended on the network path.
- It is **not** a casual per-node tweak on a **shared network**: every node that must stay on the same chain **MUST** use the **same** value; document it as an **operator contract** (see `README.md`).
- **V2** work on this flag is **documentation, warnings, and ergonomics only**—not a second, hidden validity rule. Changing the default or semantics is a **protocol version** change, not silent V2 scope.

### Handshake / session mismatch handling (as V2 adds session data)

| Case | Action |
|------|--------|
| **Local** `genesis.toml` vs **data-dir binding** (expected commitment) | **Refuse startup** — hard error, no warn-and-continue |
| **Peer** reports genesis commitment **incompatible** with local (when exchanged) | **Disconnect peer** — does not redefine block validity |
| **Peer-reported height / tip / “what to fetch next”** | **Advisory only** — used for sync scheduling; see [Sync invariant (V2)](#sync-invariant-v2) |
| **Malformed or oversized wire frames** | **Drop message and/or disconnect** per defensive policy; **log** at appropriate level |

No handshake field may imply a **new** block or transaction validity rule without a **versioned protocol** release.

### Block file recovery (`chain.blocks`)

- **Default: fail closed.** Corruption, truncation, or decode failure → **refuse** to treat the file as authoritative committed history.
- **No silent auto-repair** (no implicit truncate-to-tip, no rewrite without operator intent). Recovery is **explicit operator action** or a documented **opt-in** tool.

### `pending_tx.tril` restart semantics

- **No silent loss:** the file must not be cleared while txs were read but **not** successfully accepted into the mempool. **V2** implements **all-or-nothing drain semantics**: only clear (or rewrite) after accounting for every parsed frame—see the design note.
- **Dedup:** mempool rejects duplicate `tx_hash`; prevents double-apply of the same logical transaction.
- **Parse failure:** garbled file → **do not** partially consume; surface a **clear error** for the operator.

### Genesis / data-dir mismatch

- If binding metadata exists and disagrees with the loaded genesis document → **refuse startup** (not a warning).
- Optional **write binding** on first successful init/run is allowed in V2; behavior must be tested and documented.

### Local mempool vs consensus

- Mempool may **drop**, **bound**, or **revalidate** transactions **locally**; it must **not** cause a **consensus-invalid** transaction to appear in a sealed block.
- **Producer selection** for sealing follows [Producer-side mempool rule (V2)](#producer-side-mempool-rule-v2) below.
- **Hygiene timing (reference `run`):** In addition to the seal loop, the node runs committed-ledger hygiene **after** a successful **inbound** block append and **after** **peer catch-up** when any blocks were applied—so txs that are obsolete vs the new committed nonce (for example) are not left around solely until the next seal tick.

---

## Sync invariant (V2)

**Peer-reported tip, height, or hash is advisory only**—it informs **when and what to request**, not which chain is valid. V2 **does not** add chain selection, branch storage, competing-tip sets, fork choice, or reorg support. Synchronization remains **linear catch-up**: apply blocks that **extend the node’s single current accepted tip** under **unchanged V1** `append_block` / `try_append_network_block` rules. If a received block does not fit that tip, the node **rejects** it under existing validity/linkage rules; V2 does **not** introduce a second “preferred branch” policy.

---

## Producer-side mempool rule (V2)

Block building **stays aligned with V1 `append_block_from_mempool`**: take a **FIFO prefix** of the mempool (up to `max_transactions`), then attempt **one atomic seal**—**all** transactions in that prefix commit in **one** block **in order**, or **none** do (the mempool is **unchanged** for that failed attempt). **V2 mempool hygiene** removes stale or invalid transactions **before** this step (revalidation, explicit drops, logging). V2 **does not** add **in-seal skipping** (e.g. omitting the head transaction but including a later one in the **same** seal pass). Changing that would be a **protocol / producer** behavior change, not silent hardening.

---

## In scope

Work belongs in V2 if it fits the [classification](#classification-local-vs-wire-vs-consensus) as **local-only** or **wire-compatible but non-consensus**, and it primarily:

- improves **TCP peer behavior** (explicit session state, conservative disconnects, no new validity predicates);
- improves **linear block catch-up** (gaps, duplicates, backpressure, retries);
- improves **storage atomicity, validation on load, and restart ordering**;
- improves **observability** (logs, errors, optional status) **after** the persistence/restart design note and project decisions are in force;
- adds **defensive resource bounds** on the network path;
- improves **mempool cleanup, bounds, and revalidation** against current state **without** changing which txs are **valid in a block** under V1.

---

## Not in V2

The following remain **explicitly out of scope** (see also [Protocol freeze](#protocol-freeze-for-v2)):

- Consensus redesign, fork choice, reorgs, state snapshots over the wire.
- Chain ID in signed transactions; rich genesis; on-chain governance of parameters.
- Validator/staking/decentralization mechanics as protocol features.
- Fee destination changes; new economic rules.
- Smart contracts, bridges, multi-asset standards, throughput-first complexity.

Items under “Out of scope for V1” in `docs/genesis.md` are **later-protocol**; **chain ID** and **rich genesis** are **post–V2**, not “implied V2” (see `docs/genesis.md`).

---

## Design constraints / guardrails

- **Committed state** after applying the same blocks must match V1; V2 may not introduce **config-dependent validity**.
- **V1 block and transaction validity** remain defined in core modules; see `docs/modules.md`.
- Prefer **small, testable** changes with explicit docs over broad refactors.
- **User-focused** means: height, sync status, genesis mismatch, persistence failure, and peer drop reasons are **understandable** without leaking secrets.

---

## Prioritized implementation backlog

Checklist items are targets grouped by area; **implementation order** follows [Suggested implementation order](#suggested-implementation-order).

### Networking and peers

- [x] Document wire protocol assumptions — see [`design_notes/v2_wire_peer_sync.md`](design_notes/v2_wire_peer_sync.md).
- [x] Add or tighten **peer/session** (wire version + genesis commitment handshake; advisory height in frame only).
- [x] Improve **connection lifecycle** (shutdown, stale detection, bounded concurrency) — see [`design_notes/v2_network_defense.md`](design_notes/v2_network_defense.md).
- [x] Apply **connect timeout** and existing **read timeouts** + **frame / batch caps** on paths used by sync and gossip.

### Synchronization and catch-up

- [x] Harden **request missing blocks / heights** (linear batch validation; capped batches; multi-round catch-up).
- [x] **Backpressure / bounded work per sync call** — `SyncWorkBudget` (rounds, blocks, wire bytes) + outbound write deadlines on sync pulls; gossip cooldown unchanged (retry/backoff policy still incremental).
- [x] Extend **multi-node / sync tests** (`network_peer_sync`, updated `network_sync`, `v2_hardening`).

### Storage, persistence, and integrity

- [x] Document **on-disk invariants** (ordering, fsync, crash scenarios) — [`design_notes/v2_persistence_restart.md`](design_notes/v2_persistence_restart.md) (on-disk artifacts + fail-closed + backlog).
- [x] Reduce **partial write / detect torn frames** for **new** `chain.blocks` files: V2 magic + per-record CRC + single `write_all` per frame (legacy length-prefixed files unchanged; no auto-migration).
- [x] **Load-path validation**; fail closed per [`design_notes/v2_persistence_restart.md`](design_notes/v2_persistence_restart.md) (tests: `persistence_v2`, `restart_matrix_v2`, `storage` unit tests).

### Restart and recovery

- [x] **Cold start / replay equivalence** for the reference node (same `chain.blocks` + genesis → same state); see `node/tests/restart_matrix_v2.rs` and `persistence_v2.rs`.
- [x] Operator docs for **damaged** `chain.blocks` and **genesis mismatch** (README + [`v2_persistence_restart.md`](design_notes/v2_persistence_restart.md)).
- [x] **Pending tx** semantics across restart documented and tested (`persistence_v2`, `restart_matrix_v2`); producer rule unchanged.

### Network defense

- [x] **Ingress quotas** for expensive decode paths: `max_stale_decoded_blocks_per_session` (decoded `OP_BLOCK` with `height ≤ tip`), `max_inbound_tx_per_session` (decoded `OP_TX`); typed `PeerFrameError` disconnects (not consensus).
- [x] **Invalid decodable-block budget** per inbound session (`max_invalid_network_blocks_per_session`); stale low-height blocks do not consume that budget (separate stale quota above).
- [x] **Typed** `OP_BLOCK` append failures (`NetworkBlockPersistFailure`) and post-handshake frame errors (`PeerFrameError`) — strike/budget/disconnect decisions do not scan error-message substrings.
- [x] **Shorter `NodeInner` lock on inbound peers** — opcode classification + `decode_transaction` / `decode_block` / `GET_BLOCKS` length parse in `predecode_inbound_app_payload` **before** the mutex; tip comparison, ingress quotas, strikes, mempool submit, append/persist, and block-batch encoding **under** the lock.
- [ ] Ensure defense layers **drop work** but do not reinterpret validity.

### Mempool (local policy only)

- [x] **Purge FIFO head vs committed ledger** before seal and after failed seal (`purge_nonviable_under_committed_state`); honest policy: no queuing for future remote funding; no in-seal skipping / no future-nonce speculation; document mid-prefix atomic seal stuck case.
- [x] **Bound queue** — `--mempool-capacity` on `run` (default 10_000, max 1_000_000); **broader local drops** — `drop_stale_nonces_vs_committed` after FIFO purge; hygiene also after inbound block persist and after sync catch-up.
- [ ] No **future-nonce speculation** or producer behavior that diverges from V1 validity expectations.

### Observability and configuration

- [x] Subsystem-tagged stderr / clearer failure strings for startup, storage, sync, peer, mempool, seal, pending (`operator_msg`, `README` *Interpreting stderr*).
- [ ] Log levels and structured fields (beyond tagged eprintln).
- [x] **Key operator flags** documented in `README.md` (peer limits, drift contract, mempool capacity, persistence behavior); **consensus-sensitive** knobs called out (`--max-future-drift-secs` deployment-wide contract).

### Tests and release

- [x] **Crash/restart / corruption** library + integration coverage (`restart_matrix_v2`, `persistence_v2`, `storage` tests); full multi-node chaos remains incremental backlog.
- [x] Docs and operator runbook aligned with frozen boundary (`README.md`, this file, `v2_persistence_restart.md`, `modules.md`).

---

## Suggested implementation order

1. **Freeze the boundary** — [Protocol freeze](#protocol-freeze-for-v2), [classification](#classification-local-vs-wire-vs-consensus), [Project decisions (V2)](#project-decisions-v2), and [`design_notes/v2_persistence_restart.md`](design_notes/v2_persistence_restart.md) accepted as the planning baseline.
2. **Persistence and restart semantics** — On-disk rules, load validation, pending-tx behavior; tests first where risk is highest.
3. **Peer identity / session checks and sync hardening** — Handshake decisions, catch-up robustness, safe disconnects.
4. **Network defense limits** — Deadlines, caps, rate limits.
5. **Local mempool hygiene** — Bounds, revalidation, stale drops (no consensus drift).
6. **Observability and operator ergonomics** — Logs and CLI messages that reflect the settled behavior.
7. **Final multi-node + crash/restart test matrix** — Lock semantics before release candidate.
8. **Docs cleanup and release prep** — README, `modules.md`, `known_issues` alignment.

Observability is **early only** insofar as it helps steps 2–5; it does **not** lead the schedule ahead of persistence and sync correctness.

---

## Suggested milestones

| Milestone | Intent |
|-----------|--------|
| **V2 boundary signed off** | Protocol freeze + [project decisions](#project-decisions-v2) + persistence/restart design note locked in. |
| **V2-a: Persistence + restart** | On-disk contracts, load failure modes, pending-tx semantics, tests. |
| **V2-b: Peers + sync + defense** | Session/handshake, catch-up, resource bounds, integration tests. |
| **V2-c: Mempool + observability** | Local policy + logging/CLI aligned with frozen rules. |
| **V2 RC** | Full test matrix, docs, no open boundary ambiguities. |

---

## Notes on later versions

- **`docs/genesis.md`** — **Mismatch detection and data-dir binding** are V2 **operator safety** (with **hard startup refusal** on local mismatch when binding exists); **chain ID in txs** and **rich genesis** remain **post–V2** protocol work.
- **`docs/protocol_overview.md`** — Fee destination, speculative mempool, and consensus upgrades beyond linear V1 are **not V2** unless separately versioned later.
- **`docs/vision.md`** — Stronger consensus and decentralization are **V3+**; V2 must not absorb them.

---

## Related documents

- `docs/design_notes/v2_persistence_restart.md` — persistence, `pending_tx.tril`, recovery, startup vs peer actions.
- `docs/design_notes/v2_wire_peer_sync.md` — TCP session handshake, linear sync batch rules, defensive caps.
- `docs/vision.md` — long-term version ladder.
- `docs/v1_scope.md` — frozen V1 feature set.
- `docs/genesis.md` — genesis rules and later-protocol candidates.
- `docs/protocol_overview.md` — V1 behavior and directional notes.
- `docs/architecture.md` — module layout.
- `docs/modules.md` — consensus boundary and V2 constraints on config.
