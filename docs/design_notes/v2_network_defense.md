# V2 design note: inbound network defense and connection lifecycle

Local-only hardening for the existing TCP peer server. Does **not** change block/tx validity, fork choice, or wire opcodes. Complements [`v2_wire_peer_sync.md`](v2_wire_peer_sync.md).

**Operator stderr:** The reference `node` tags inbound/outbound session messages with `[peer]` (and related prefixes). See `README.md` → *Interpreting stderr* and the `operator_msg` module in `node`.

**Invalid-block strikes:** Whether an inbound `OP_BLOCK` failure counts toward `max_invalid_network_blocks_per_session` is decided from the typed `NetworkBlockPersistFailure` in the `network` module, not substring matching on error text — operator-facing strings can change without altering strike/disconnect behavior.

**Ingress work quotas (expensive decode paths):** Separate per-session caps bound (1) decoded `OP_BLOCK` with `height ≤ local_tip` (stale replay decode/hash work — does **not** use invalid-block strikes) and (2) successfully decoded inbound `OP_TX` (each may run `basic_validate` / mempool admit). Exhaustion disconnects with stable tags `TRIL_INGRESS_STALE_BLOCK_QUOTA_EXHAUSTED` / `TRIL_INGRESS_INBOUND_TX_QUOTA_EXHAUSTED` via typed `PeerFrameError` variants — local-only, non-consensus.

**Lock scope:** Post-handshake frame bodies are opcode-classified and `decode_transaction` / `decode_block` / `GET_BLOCKS` length parsing run in `predecode_inbound_app_payload` **before** acquiring the global `NodeInner` mutex; only poison checks, tip comparison, mempool submit, append/persist, and block batch encoding run under the lock.

---

## Lifecycle (inbound)

1. **Accept** → if concurrent session count ≥ configured cap, the socket is **dropped** immediately (no worker thread).
2. Otherwise a worker thread holds a **slot** from accept until the session ends (handshake failure, idle timeout, policy limit, or clean EOF). **The slot covers the whole session, including V2 handshake** — a peer that opens TCP but never finishes HELLO still consumes capacity until timeout or close (same class of issue as post-handshake idle).
3. **Handshake** uses the same V2 HELLO / HELLO_ACK as today; timeouts are the socket read/write deadlines below.
4. **Post-handshake loop** reads framed messages until EOF, **idle read timeout**, **max application frames**, **max post-handshake protocol errors**, **invalid next-height block budget**, or **stale-block / inbound-tx ingress quotas**.
5. **Oversized wire frame** (length header over cap) → **immediate** disconnect (fail closed), not counted against the soft protocol-error budget.

---

## Defaults (see also `node run --help` / README)

| Knob | Default | Role |
|------|---------|------|
| `max_concurrent_sessions` | 128 | Inbound cap |
| `idle_read_timeout` | 120s | No full frame in time → close |
| `write_timeout` | 60s (`0` = off in CLI) | Slow reader / blocked write |
| `max_protocol_errors_per_session` | 32 | Unknown opcode, bad lengths, etc. |
| `max_app_frames_per_session` | 100_000 | Hard stop on spammy sessions |
| `max_invalid_network_blocks_per_session` | 24 | Decodable `OP_BLOCK` with `height > tip` that fails append for peer-invalid reasons (`NetworkBlockPersistFailure::PeerRejectedBlock`) |
| `max_stale_decoded_blocks_per_session` | 8192 | Decodable `OP_BLOCK` with `height ≤ tip` (no append); bounds stale replay decode spam |
| `max_inbound_tx_per_session` | 100_000 | Successfully decoded `OP_TX` per session |

CLI: `--peer-max-stale-blocks`, `--peer-max-inbound-tx` (see `README`).

---

## Outbound TCP (sync client + gossip)

- **Connect timeout:** `PEER_CONNECT_TIMEOUT_SECS` (10s).
- **Read / write deadlines:** `OutboundPeerTimeouts` in `node` — default read 90s, write **60s** (`PEER_OUTBOUND_WRITE_TIMEOUT_SECS`). Applied on every outbound peer socket used for `pull_blocks_from_peer`, `push_block_to_peer`, and `push_tx_to_peer` (after connect, before handshake writes).
- **Purpose:** a peer that accepts TCP and completes the session handshake but **never drains** reads must not wedge the node indefinitely on `write_all` / `flush`. Failures are local I/O errors only; no consensus meaning.
- **Overrides:** `push_block_to_peer_with_timeouts` / `push_tx_to_peer_with_timeouts` for tests or custom tooling.

---

## Outbound gossip (binary `run`)

After **five** consecutive block-push failures to the same `--peers` entry, the node applies a **45s local cooldown** before retrying that peer. Delivery-only; does not change validity.

---

## Classification

All of the above is **operational policy** per [`v2_scope.md`](../v2_scope.md) (connection limits, deadlines, local backoff).
