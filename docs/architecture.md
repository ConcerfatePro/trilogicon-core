# Trilogicon Architecture

## Overview

Trilogicon is implemented as a Rust-based node with clear module boundaries.

The architecture should remain:

- simple
- understandable
- testable
- modular
- safe to evolve

---

## Suggested repository structure

```text
Trilogicon/
├─ docs/
│  ├─ vision.md
│  ├─ v1_scope.md
│  ├─ v2_scope.md
│  ├─ design_notes/
│  │  └─ v2_persistence_restart.md
│  ├─ protocol_overview.md
│  ├─ design_principles.md
│  ├─ protocol_invariants.md
│  ├─ architecture.md
│  ├─ change_policies.md
│  └─ roadmap.md
└─ node/
   ├─ Cargo.toml
   └─ src/
      ├─ main.rs
      ├─ lib.rs
      ├─ types.rs
      ├─ errors.rs
      ├─ crypto.rs
      ├─ transaction.rs
      ├─ block.rs
      ├─ state.rs
      ├─ blockchain.rs
      ├─ mempool.rs
      ├─ consensus.rs
      ├─ network.rs
      ├─ storage.rs
      └─ wallet.rs
```

---

## Module responsibilities (V1-oriented)

- `types.rs`: shared protocol data types (addresses, accounts, aliases).
- `errors.rs`: typed protocol and validation errors.
- `crypto.rs`: hashing, signature verification, key/address helpers.
- `transaction.rs`: transaction model, canonical payload logic, tx validation.
- `block.rs`: block model and block-level structural checks.
- `state.rs`: deterministic account state transitions and invariants.
- `blockchain.rs`: chain append logic and canonical chain checks.
- `mempool.rs`: pending transaction intake and bounded storage.
- `consensus.rs`: minimal V1 block production/acceptance policy.
- `network.rs`: peer messaging, block/tx propagation, sync flow.
- `storage.rs`: `chain.blocks` persistence (V2 CRC-framed records for new files; legacy frames supported), `load_blockchain_from_disk`, in-process append poison handling.
- `wallet.rs`: local key management and transaction signing helpers.

---

## Architectural boundaries

- Protocol validation does not depend on networking internals.
- State transition logic is deterministic and testable in isolation.
- Storage is abstracted so in-memory and persistent backends can coexist.
- Consensus checks are explicit and independently verifiable by each node.

These boundaries keep V1 auditable while leaving room for evolution; planned V2 work is scoped in [`v2_scope.md`](v2_scope.md), broader consensus and performance work remains for later versions (`docs/vision.md`).
