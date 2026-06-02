# Architecture Notes

Trilogicon is a single Rust reference node plus a small optional dev UI.

## Node shape

- `transaction.rs`: signed transfers, hashing, and signature checks.
- `state.rs`: account balances, nonces, and transaction application.
- `block.rs`: block structure and block-hash validation.
- `blockchain.rs`: the live linear chain, append rules, state replay, and block sealing.
- `storage.rs`: local `chain.blocks` persistence and reload.
- `network.rs`: TCP framing, peer sessions, gossip, and linear catch-up.
- `mempool.rs`: local queue policy for pending transactions.
- `genesis.rs`: shared genesis allocation file and commitment.

## Current model

V1/V2 keep one committed tip. Blocks either extend that tip or are rejected by the live node. There is no live branch store, fork choice, or reorg execution yet.

V3 files under `node/src/v3/` are design scaffolding for future branch/index/reorg work. They are library-local and inert.

## Design bias

Prefer boring, explicit behavior: fail closed on corrupt local data, keep wire messages bounded, and make operator errors visible.
