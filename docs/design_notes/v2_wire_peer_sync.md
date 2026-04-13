# V2 design note: wire session and linear sync

Operational contract for **peer/session hardening** and **linear catch-up** on the existing V1 TCP framing. Does **not** add fork choice, branch storage, or new block/tx validity rules. See [`v2_scope.md`](../v2_scope.md) (sync invariant, handshake table).

---

## Session handshake (mandatory first exchange)

After TCP connect, the **initiator** sends one framed payload starting with **`OP_SESSION_HELLO` (5)**. The **responder** must reply with one framed payload starting with **`OP_SESSION_HELLO_ACK` (6)** before any other opcodes. If genesis **state commitment** (UTF-8 hex, same as `genesis.toml` / data-dir bind) does not match exactly, the peer **disconnects** (fail closed). This is **compatibility only** — it does not redefine valid blocks.

Frame body (both HELLO and HELLO_ACK):

| Field | Size | Meaning |
|-------|------|---------|
| opcode | 1 | `5` = HELLO, `6` = ACK |
| `wire_version` | 2 BE | Must equal **`TRIL_WIRE_PROTOCOL_VERSION`** (2 in this milestone) |
| `commitment_len` | 2 BE | Byte length of UTF-8 hex string |
| `commitment_hex` | `len` | `Genesis::state_commitment_hex()` |
| `advisory_height` | 8 BE | **Advisory only** — local chain height at send time; **must not** be used to choose branches or replace committed history |

The frame body length must be **exactly** `1 + 2 + 2 + commitment_len + 8` bytes (no trailing bytes after `advisory_height`).

Reference node logs peer `advisory_height` at **debug/diagnostic** level only (implementation may use `eprintln!` with explicit “advisory” wording).

---

## Post-session messages

Unchanged opcode meanings: `OP_TX` (1), `OP_BLOCK` (2), `OP_GET_BLOCKS` (3), `OP_BLOCKS` (4). Length-prefixed outer frame cap is **`MAX_WIRE_FRAME_BYTES`** on **both** read and write: implementations must not emit a frame body larger than this cap (fail closed). **`OP_BLOCKS`** inner batch is additionally capped at **`MAX_BLOCKS_PER_BATCH`** blocks per response, and encoded batch bodies must still fit under **`MAX_WIRE_FRAME_BYTES`**.

---

## Linear sync (catch-up)

- Sync requests blocks starting at **`local_height + 1`** only — never derived from peer advisory height.
- The follower **validates the entire batch** as a strict linear extension (contiguous heights, `previous_hash` links to prior tip or prior batch entry, `basic_validate` on each block) **before** appending any block.
- **`sync_from_peer` loops**: each iteration opens a connection (handshake + `GET_BLOCKS`), applies at most **`MAX_BLOCKS_PER_BATCH`** blocks per response, then repeats until the peer returns an **empty** batch (caught up), or a **local** `SyncWorkBudget` (in `node`, `network.rs`) stops the call by capping rounds, appended blocks, or total bytes read from responses in that **single** invocation. Stopping on a budget is operational only; the next normal sync attempt continues from `local_height + 1` with the same linear rules.
- **Byte budget (soft):** if one `OP_BLOCKS` body is larger than `max_wire_bytes_per_call`, the node still **validates and appends** that batch (subject to the block cap), then ends the call with `stopped_due_to_budget` so the next invocation starts with a **fresh** byte counter—avoiding a permanent no-progress loop. Budget fields must be **≥ 1** (`validate_sync_work_budget`); use `u64::MAX` for an effectively unlimited byte cap.
- **Clock on the network path:** each appended block uses a **fresh** local `now_unix` sample (not one value captured at the start of a long sync), so `max_future_drift_secs` is not accidentally evaluated against a stale clock during catch-up.
- Each block still goes through `append_network_block_persist` (unchanged V1 ingress path including timestamp drift).
- Duplicate / stale / gapped / out-of-order batches are **rejected** with no append from that batch.

---

## Defensive limits (local / operational)

- Connect timeout (default 10s) on outbound sync/gossip.
- Read timeout on peer streams (existing behavior, documented).
- `MAX_BLOCKS_PER_BATCH` limits work per `GET_BLOCKS` reply (the node collects at most that many blocks from local storage before encoding; it does not clone the entire chain suffix).
- Malformed or oversized session or batch payloads → disconnect + log.

These bounds are **not** consensus rules; they only bound resource use and I/O.
