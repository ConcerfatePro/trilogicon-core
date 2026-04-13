# Trilogicon (TRIL)

Minimal V1 value-transfer chain in Rust: accounts, signed transfers, blocks, deterministic replay protection, shared **genesis**, and basic TCP sync between nodes.

## Current release-candidate phase

The current stabilization branch is:

`release/v1.0.0-rc1`

V1 is feature-frozen. The current focus is bug fixing, testing, documentation, operator experience, and private technical validation.

Design rules live under [`docs/`](docs/) (scope, invariants, architecture). **Genesis** is documented in [`docs/genesis.md`](docs/genesis.md). **V1 protocol semantics are frozen**; **V2** in this repository is a **node-hardening** line (persistence, sync operability, operator messaging, local resource policy) scoped in [`docs/v2_scope.md`](docs/v2_scope.md) without changing validity rules. Module ownership and the consensus boundary are summarized in [`docs/modules.md`](docs/modules.md); the V1 completion checklist is [`docs/v1_checkpoint.md`](docs/v1_checkpoint.md). For fmt/clippy/test and PR expectations, see [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Troubleshooting

### `wallet.seed already exists`
`init` will not overwrite an existing wallet. Use a fresh data directory or remove the old test directory first.

### `genesis file not found`
The node expects `genesis.toml` in the data directory unless `--genesis PATH` is provided. All nodes in the same network must use the same shared genesis.

### Nodes do not match
Check that both nodes use the same merged `genesis.toml`. Compare genesis state commitment if needed.

### Second `node run` on the same `--data-dir` exits immediately
Only one `run` process may use a data directory at a time. The node creates `.node.run.lock` and holds an **exclusive** lock for the process lifetime. A second `run` prints a **`[startup]` fail-closed** message naming the lock file and exits; this is **not** corruption. `send` does **not** take this lock so pending-tx appends still work while a node is running.

### `session handshake` / `genesis commitment mismatch` on connect
V2 TCP peers exchange a short **session handshake** first (wire version + genesis state commitment). Mismatch **disconnects** the peer; it does not change block validity rules. Fix: align `genesis.toml` (and data-dir bind) across nodes. Details: [`docs/design_notes/v2_wire_peer_sync.md`](docs/design_notes/v2_wire_peer_sync.md).

### Inbound peer limits (`run`)
Optional **`node run`** flags tune **local** connection policy only (not consensus): `--max-inbound-peers` (default 128), `--peer-idle-timeout-secs` (default 120), `--peer-write-timeout-secs` (default 60; `0` disables), `--peer-max-wire-errors`, `--peer-max-frames`, `--peer-max-stale-blocks` (decoded stale `OP_BLOCK` quota per session), `--peer-max-inbound-tx` (decoded `OP_TX` quota per session), `--mempool-capacity` (in-memory tx cap; see below). Oversized frames still disconnect immediately. See [`docs/design_notes/v2_network_defense.md`](docs/design_notes/v2_network_defense.md).

### Mempool capacity (`run`)

`--mempool-capacity N` sets the **maximum number of transactions** the in-memory mempool will hold at once.

- **Default:** `10000`
- **Maximum:** `1000000` (hard cap to bound RAM use)
- **Minimum:** `1`
- **Local-only:** This is an **operator / resource** knob. It does **not** change which transactions or blocks are valid under the protocol, and it does **not** need to match across peers. If the pool is full, new submissions are rejected until space is freed by sealing, hygiene drops, or manual process restart.

### `chain.blocks` on disk (V2 vs legacy)

The node still replays the **same** canonical block bytes as before; only the **local file framing** differs by age of the file:

- **New or empty files:** The first successful append writes an 8-byte magic header (`TRILBC01`), then each stored block is `length + encode_block(bytes) + CRC-32` of that payload. Load validates the CRC so torn or corrupted frames fail **closed** (startup error), not half-applied.
- **Existing “legacy” files** (no magic header): Still supported as `length + encode_block(bytes)` per record, as in earlier releases. The node does **not** rewrite or auto-migrate legacy files to the V2 layout.
- **Truncated / corrupt files:** Startup **refuses** to load; repair or replace the file (see [`docs/design_notes/v2_persistence_restart.md`](docs/design_notes/v2_persistence_restart.md)).

### Pending queue (`pending_tx.tril`) — practical notes

- **`send`** appends one length-prefixed transaction frame under `.pending_tx.lock` (advisory lock in the data directory).
- **`run`** reads and parses the whole file, admits txs to the mempool in order, then **atomically** replaces the file with whatever could not be admitted (or clears it if all were accepted). The temp file is **fsync**’d before rename; on Unix the parent directory is **fsync**’d after rename when possible (best-effort durability of the rename).
- **Parse errors:** The file is **not** partially consumed; fix or remove it with care. See the design note for drain vs mempool consistency.

### When mempool “hygiene” runs (local policy)

The node periodically **drops** transactions that are no longer sensible against the **already committed** ledger (for example stale nonce at the FIFO head, or globally obsolete nonces after the ledger moved). **`try_submit` also rejects a second distinct transaction with the same `(sender, nonce)`** as an already queued tx (first FIFO entry wins); this is **local admission only** and does not change which txs are valid on-chain. This is **not** a consensus rule: two honest nodes can still have different mempools; it only affects what **this** process might seal next.

Hygiene runs:

1. **Each `run` loop tick** — before attempting to seal (and again after a failed seal attempt).
2. **After a successful inbound block** from a peer (persisted to `chain.blocks`).
3. **After outbound peer catch-up** (`sync_from_peer`) when at least one block was appended.

If your ledger advances via sync or gossip, transactions with `nonce` **strictly below** the sender’s committed nonce may disappear from the local mempool; re-submit if you still need them (they would be invalid on-chain anyway).

### `--peers` and block sealing (no fork-choice repair in V2)

If you pass **`--peers`** with at least one address, the node **disables local sealing** until **every** configured peer has returned success from at least one **initial** outbound catch-up (`sync_from_peer`). On each run-loop tick it **retries** failed peers. Until all succeed, stderr logs a **`[sync]`** warning that sealing is off and that operating on a stale tip is risky when peers are configured but unreachable. There is **no reorg / fork-choice repair** in V2—this is an operator safety gate only. **Solo producers** should omit `--peers` (or ensure peers are up) if you need empty blocks sealed on schedule.

### `send` succeeded but no block appeared
`send` only queues the transaction. A running `run` process must pick it up, seal a block, and gossip it to peers.

### Data directory: chain file, binding, pending queue, restart
- **`chain.blocks`:** Append-only block history (genesis block is **not** in the file). New chains use the V2 magic + CRC framing; legacy files remain readable. If this file is **truncated or corrupt**, the node **refuses startup** until you repair or replace it.
- **`.node.run.lock`:** Exclusive lock for **`node run`** only; prevents two `run` processes from sharing one data directory. Released when the process exits.
- **`genesis_bind.toml`:** If the file exists, it is **verified before** `chain.blocks` is loaded. If it is **missing**, it is **created only after** a successful chain load (`run` / `send`). **Mismatch** with `genesis.toml` → **refuse startup**. A failed startup after verification therefore does **not** leave a new bind file behind. Deleting `genesis_bind.toml` to force re-bind is **only** appropriate **before** `chain.blocks` has any committed history (still genesis-only). If `chain.blocks` already has blocks, **do not** casually re-bind that directory to a different genesis—use a **fresh data directory**, or an **intentional chain reset** (remove/replace `chain.blocks`, then re-sync) with full awareness. See [`docs/design_notes/v2_persistence_restart.md`](docs/design_notes/v2_persistence_restart.md).
- **`pending_tx.tril`:** If **garbled**, `run` logs a drain error and **does not** silently wipe the file; fix or delete under guidance.
- **Restart:** Reloading disk replays the same committed history deterministically. In-process “poisoned” block store state does **not** survive restart, but a **partially written** chain file still fails at load. Full detail: [`docs/design_notes/v2_persistence_restart.md`](docs/design_notes/v2_persistence_restart.md).

### Interpreting stderr (reference `node`)

The binary prefixes most diagnostic lines on stderr so you can see **which subsystem** failed and whether to treat it as **local**, **peer-scoped**, or a **bounded retry**:

| Prefix | Typical meaning |
|--------|-----------------|
| `[startup]` | Data-dir setup: genesis binding, initial `chain.load`, process banner. |
| `[storage]` | `chain.blocks` I/O; **fail-closed** refusal to load; **poisoned** append state for **this process only** after a write/sync failure. |
| `[sync]` | Outbound catch-up; **bounded stop** when the line mentions `stopped_due_to_budget` (per-call cap; next sync continues from current height). |
| `[peer]` | TCP session (handshake, malformed wire, caps, idle timeout); gossip errors are usually **remote reachability**, not local disk corruption. |
| `[mempool]` | Local hygiene vs the **committed** ledger (FIFO head cleanup, stale-nonce drops after ledger moves, duplicate **(sender, nonce)** drops, capacity rejects). |
| `[seal]` | Local block production: committed height, or seal attempt failed without persisting a new block. |
| `[pending]` | `pending_tx.tril` lock/drain; any drain error logs here — guarantees depend on failure type (design note). |

**Fail-closed:** The node stops rather than guess (corrupt chain file, bind mismatch, unrecoverable persist). Repair or restore per [`docs/design_notes/v2_persistence_restart.md`](docs/design_notes/v2_persistence_restart.md).

**Poisoned store:** After an append/sync error, this process refuses further `chain.blocks` appends until exit; restart after the file is consistent. The flag is **not** stored on disk.

Inbound limits and strike budgets are documented in [`docs/design_notes/v2_network_defense.md`](docs/design_notes/v2_network_defense.md).

## Build

```bash
cd node
cargo build
cargo test
```

Integration tests include subprocess **two-node**, **restart**, **three-node fan-out** E2Es (`node/tests/cli_*_e2e.rs`), and a **run lock** E2E (`node/tests/run_data_dir_lock_e2e.rs`). The `node` binary is at `node/target/debug/node` (or `release` with `--release`).

## Operator runbook (local two-node)

Every machine in a network must use the **same** `genesis.toml` (compare **genesis state commitment** lines when in doubt).

### 1) First node — wallet + genesis for one address

```bash
cd node
cargo run -- init --data-dir ./data-a --genesis-balance 10000000
```

This creates:

- `data-a/wallet.seed`
- `data-a/genesis.toml` (initially with one allocation)

It also prints:

- **Address:**
- **Genesis state commitment:**

Note:
- `wallet.seed` is secret and should never be committed.
- If `wallet.seed` already exists in that data directory, `init` will refuse to overwrite it. Use a fresh directory or remove the old test directory first.

### 2) Second node — wallet only

```bash
cargo run -- init --data-dir ./data-b
```

This creates:

- `data-b/wallet.seed`

Note the printed **Address:** for party B.

### 3) Merge allocations into one shared genesis

Edit **one** canonical `genesis.toml` so it contains **both** `[[allocations]]` entries (party A and party B), with the balances you want. You can start from `node/examples/genesis.template.toml`.

Rules:

- Use the same final file on **every** node.
- Same logical allocations should produce the same **genesis state commitment**.
- No duplicate `address` lines.
- Balances must be `> 0`.
- Addresses must be valid (non-empty, `<= 128` chars).

Copy that merged file to:

- `data-a/genesis.toml`
- `data-b/genesis.toml`

Overwriting `data-a/genesis.toml` after merge is expected.

Important:
- If you manually edit a genesis file that already contains an old printed commitment comment, remove that old comment unless it has been regenerated to match the new merged allocations.

### 4) Run two nodes (two terminals)

**Terminal B** (listener first):

```bash
cargo run -- run --data-dir ./data-b --listen 127.0.0.1:9334 --interval-secs 2
# Optional: cap mempool size (local only), e.g. `--mempool-capacity 5000`
```

You should see something like (stdout / stderr use **`[peer]`** / **`[startup]`** prefixes):

```text
[peer] listening on 127.0.0.1:9334
[startup] mempool capacity 10000 tx slots (local bound; not consensus)
[startup] Trilogicon node | height=0 | wallet=...
[startup] Ctrl+C to stop.
```

**Terminal A** (producer + gossip to B):

```bash
cargo run -- run --data-dir ./data-a --listen 127.0.0.1:9333 --peers 127.0.0.1:9334 --interval-secs 2
```

You should see something like:

```text
[peer] listening on 127.0.0.1:9333
[startup] mempool capacity 10000 tx slots (local bound; not consensus)
[startup] Trilogicon node | height=0 | wallet=...
[startup] Ctrl+C to stop.
```

With **`--peers`**, stderr may also include **`[sync]`** lines (for example **`+N block(s) appended from …`** after catch-up, or a fail-closed warning if a peer is unreachable). If initial catch-up fails for any peer, sealing stays off until every peer succeeds (retried each loop); after recovery you may see **`[sync] all configured peers completed catch-up — local sealing enabled`**. See **Troubleshooting** above.

### 5) Submit a transfer

```bash
cargo run -- send --data-dir ./data-a RECEIVER_ADDRESS AMOUNT [FEE]
```

Use **B’s address** as `RECEIVER_ADDRESS`.

Example:

```bash
cargo run -- send --data-dir ./data-a <B_ADDRESS> 100 1
```

`send` does **not** directly create a block. It writes the transaction to `pending_tx.tril`. The running `run` loop on node A later picks it up, seals it into a block, and gossips that block to peers.

You should see output similar to:

```text
Queued tx ... -> RECEIVER amount 100 fee 1 (nonce 0)
```

Then on the running node process, something like:

```text
[seal] committed height=1 with 1 transaction(s)
[peer] session ok (outbound; wire v2; peer advisory height 1 — advisory only, not used for sync bounds)
```

### 6) Optional: wall-clock drift limit on **inbound** blocks (`--max-future-drift-secs`)

This flag adjusts how far **ahead** a received block’s timestamp may be relative to the **local** system clock before the node rejects it on the **network ingress** path. That affects whether a block is **accepted into local state**—it is **consensus-relevant behavior**.

- **Operator contract:** on any deployment where nodes must share one chain, **every node MUST use the same `--max-future-drift-secs` value** (or all omit it and rely on the same default). **Do not** tune this independently per machine in production-style networks.
- **Default:** omit the flag to use the node’s built-in default (see `consensus` / CLI help).
- **Use cases:** **tests**, **debugging**, or **homogeneous** fleets with an explicit, shared configuration.

V2 scope and protocol-freeze boundaries: [`docs/v2_scope.md`](docs/v2_scope.md) ([Protocol freeze](docs/v2_scope.md#protocol-freeze-for-v2), [Project decisions (V2)](docs/v2_scope.md#project-decisions-v2)).

Example (illustrative only):

```bash
cargo run -- run --data-dir ./data-b --listen 127.0.0.1:9334 --peers 127.0.0.1:9333 --interval-secs 2 --max-future-drift-secs 900
```

### 7) Optional: restart sanity check

After at least one successful transfer:

- Stop both nodes with `Ctrl+C`
- Restart them with the same commands
- Confirm they reload the same height from disk

If restart worked correctly, both nodes should come back with the same persisted height and remain in sync.

## Files under a data directory

| File | Role |
|------|------|
| `wallet.seed` | 32-byte secret seed (back up; never commit) |
| `genesis.toml` | Protocol height-0 allocations (must match across the network) |
| `genesis_bind.toml` | V2: records the genesis **state commitment** for this data dir; verified before chain load if present; **created only after** a successful `chain.blocks` load on first `run` / `send`. If you change `genesis.toml`, deleting `genesis_bind.toml` to re-bind is **only** appropriate **before** `chain.blocks` holds committed block history. Once history exists, use a **new data dir** or **reset/re-sync** the chain—**not** a casual bind delete. |
| `.node.run.lock` | Exclusive lock file for **`node run`** (empty while held locked by the kernel / `fs2`; not human-edited) |
| `chain.blocks` | Persisted non-genesis blocks (V2 CRC framing for new files; legacy length-prefixed frames still loaded) |
| `pending_tx.tril` | Queue written by `send`, consumed by `run` (lock + atomic rewrite; see above) |

## CI / tests

- Library and integration tests: `cargo test` from `node/`
- A subprocess two-node test (`tests/cli_two_node_e2e.rs`) spawns the real `node` binary and runs automatically with `cargo test`
- Restart, three-node fan-out, and run-directory lock E2Es also run under `cargo test`

## Scope

V1 is intentionally narrow:

- **no** smart contracts
- **no** fork-choice / reorg tooling in the current operator flow
- **no** state snapshots over the wire

Nodes replay persisted **blocks** on top of shared genesis. See [`docs/v1_scope.md`](docs/v1_scope.md). **V2** node-hardening scope and freeze rules are in [`docs/v2_scope.md`](docs/v2_scope.md).
