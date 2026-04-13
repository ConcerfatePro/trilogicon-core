
# Trilogicon Roadmap

## Overview

This roadmap records how the project was phased historically (**V1** foundation) and where responsibility sits now.

- **V1** — Secure value-transfer baseline: accounts, signed transfers, blocks, deterministic validation, basic sync. **Implementation complete** per [`v1_checkpoint.md`](v1_checkpoint.md).
- **V2** — Node hardening on the **same** V1 protocol: persistence, restart safety, TCP session/sync robustness, local mempool policy, operator messaging. **Reference node complete** per [`v2_checkpoint.md`](v2_checkpoint.md); scope definition in [`v2_scope.md`](v2_scope.md).
- **V3+** — **Planning** lives in [`v3_scope.md`](v3_scope.md); **not** the active implementation track on `main` until approved. Code that changes consensus or fork choice requires an explicit protocol delta and checklist—not silent `main` drift.

The goal is not to rush features; it is to keep **honest boundaries** between protocol versions and node implementation quality.

## Current progress snapshot (main)

- **Protocol core (V1):** Frozen for the V1 scope; see [`v1_scope.md`](v1_scope.md).
- **Reference node (V2):** Hardening line merged and stabilized on `main`; CI runs `fmt`, `clippy -D warnings`, and `cargo test` on Linux, Windows, and macOS (see [`v2_checkpoint.md`](v2_checkpoint.md)).
- **After V2.0.0 tag:** Maintenance per [`v2_freeze.md`](v2_freeze.md); release notes [`releases/v2.0.0.md`](releases/v2.0.0.md). **Next decisions:** V2.1 polish, larger test matrices, or **V3** implementation (only after [`v3_scope.md`](v3_scope.md) + chain-rules spec).

The **phase sections below** remain as a **historical** map of how V1 was built; they are not a literal “current sprint board.”

---

## Phase 0 - Project framing and protocol definition

### Goals

- define project identity clearly
- define V1 scope
- define design principles
- define protocol invariants
- define architecture direction
- define success criteria

### Deliverables

- `vision.md`
- `v1_scope.md`
- `protocol_overview.md`
- `design_principles.md`
- `protocol_invariants.md`
- `architecture.md`
- `change_policies.md`

---

## Phase 1 - Core data model and cryptographic foundations

### Goals

- define core protocol types
- define address representation
- define hashing rules
- define signing and verification interfaces
- create wallet/account basics

### Deliverables

- `types.rs`
- `errors.rs`
- `crypto.rs`
- `wallet.rs`

### Exit criteria

- addresses can be represented consistently
- hashes are deterministic
- signing flow is defined
- signature verification path is working

---

## Phase 2 - Transaction layer

### Goals

- implement transaction structure
- define canonical transaction payload rules
- implement hashing and validation
- implement signature verification path
- implement nonce checks

### Deliverables

- `transaction.rs`
- transaction tests

### Exit criteria

- valid transactions pass
- malformed transactions fail
- invalid signatures fail
- replayed/stale nonce transactions fail

---

## Phase 3 - Block layer

### Goals

- implement block structure
- implement block hashing
- implement block-level validation
- define transaction ordering assumptions

### Deliverables

- `block.rs`
- block validation tests

### Exit criteria

- blocks are linked correctly
- invalid block format fails
- invalid transaction sets inside blocks fail

---

## Phase 4 - State transition engine

### Goals

- implement account state storage
- implement deterministic transaction application
- enforce balance and nonce rules
- ensure invalid transactions do not mutate state

### Deliverables

- `state.rs`
- state transition tests

### Exit criteria

- balances update correctly
- nonces update correctly
- invalid operations do not partially apply
- repeated application is deterministic

---

## Phase 5 - Blockchain validation and chain management

### Goals

- implement genesis handling
- implement block append logic
- implement full chain validation
- ensure final state consistency

### Deliverables

- `blockchain.rs`
- chain validation tests

### Exit criteria

- chain can be replayed from genesis
- invalid chains are rejected
- valid chains reconstruct correct state

---

## Phase 6 - Basic consensus / block production model

### Goals

- implement a simple and safe V1 block production mechanism
- keep consensus logic intentionally modest
- ensure all blocks remain independently verifiable

### Deliverables

- `consensus.rs`

### Exit criteria

- nodes can determine block validity consistently
- block production model supports controlled testing
- consensus logic does not undermine determinism

---

## Phase 7 - Networking and synchronization

### Goals

- connect nodes
- propagate transactions
- propagate blocks
- request missing blocks
- sync state through chain replay or accepted chain flow

### Deliverables

- `network.rs`
- basic multi-node test environment

### Exit criteria

- multiple nodes can exchange blocks/transactions
- nodes can catch up from peers
- honest nodes converge on the same valid chain

---

## Phase 8 - Storage and reliability improvements

### Goals

- improve persistence
- make restart behavior more reliable
- prepare for more realistic testing

### Deliverables

- `storage.rs`
- persistence tests

### Exit criteria

- node can persist required data
- restart behavior is consistent
- stored data does not break validation assumptions

---

## Phase 9 - Hardening and pre-testnet stabilization

### Goals

- improve test coverage
- improve error handling
- improve logging and debuggability
- review edge cases and invariants
- reduce fragile assumptions

### Deliverables

- improved tests
- bug fixes
- cleanup pass across core modules

### Exit criteria

- core invariants hold under testing
- multi-node behavior is stable enough for an early testnet
- V1 rules are documented clearly enough for future continuation

---

## V1 done means

V1 should be considered complete when:

- the rules are clear
- the implementation is coherent
- transfers work correctly
- validation is strict
- replay protection works
- blocks and chains validate correctly
- multiple nodes can synchronize
- the codebase is understandable enough to build V2 safely (see [`v2_scope.md`](v2_scope.md))