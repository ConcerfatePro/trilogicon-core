# V2 Scope

V2 is node hardening for the existing linear V1 protocol.

It does not change what a valid transaction or block means. It makes the reference node less fragile around local storage, restarts, peer sessions, sync, mempool cleanup, and operator diagnostics.

## Frozen protocol areas

V2 must not change:

- transaction payloads, hashes, signatures, or sender binding;
- block structure or block-hash rules;
- fee burn;
- nonce and balance application;
- genesis model;
- linear extension only;
- lack of chain ID in signed payloads;
- lack of staking, rewards, fork choice, or reorgs.

If a config flag would make two honest nodes commit different state from the same genesis and same blocks, it is not a V2 hardening change.

## What V2 adds

- V2 `chain.blocks` framing for new files: magic header plus per-frame CRC.
- Legacy `chain.blocks` files still load; no auto-migration.
- Fail-closed startup for corrupt, truncated, undecodable, or replay-invalid chain files.
- Narrow repair for a 1-3 byte incomplete length-prefix tail after at least one complete frame.
- `genesis_bind.toml` data-dir binding.
- `pending_tx.tril` drain semantics that avoid silent loss.
- TCP handshake with wire version and genesis commitment.
- Bounded peer/session work and typed disconnect reasons.
- Linear sync catch-up with caps and budget stops.
- Local mempool capacity and cleanup against committed state.
- Subsystem-tagged stderr messages.

## Local policy vs consensus

Local policy may reject, delay, or drop work before it reaches consensus validation. Examples: peer caps, idle timeouts, mempool capacity, stale queued tx cleanup, and malformed-frame disconnects.

Consensus-sensitive behavior includes block validity, transaction validity, state application, and timestamp admission on the network path. Shared deployments should use the same `--max-future-drift-secs` value.

## Sync invariant

Peer-reported height, tip, or hash is advisory. V2 sync still applies only blocks that extend the node's single current tip through the normal append path. Competing branches are not stored or selected.

## Producer mempool rule

The local producer scans queued transactions in FIFO order against a cloned state and selects an executable subsequence. It may skip currently non-executable entries, but it must not reorder selected transactions or include a transaction before its nonce is valid. Final block append remains atomic.

## Out of scope

- fork choice, reorgs, side-branch storage;
- state snapshots over the wire;
- chain ID in signed transactions;
- fee routing, rewards, staking, governance;
- smart contracts or application features.

## Release state

V2 is complete for the reference node. After the `v2.0.0` tag, V2 behavior is frozen under [`v2_freeze.md`](v2_freeze.md). See [`v2_checkpoint.md`](v2_checkpoint.md) and [`releases/v2.0.0.md`](releases/v2.0.0.md).
