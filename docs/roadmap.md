
# Trilogicon Roadmap

## Overview

This roadmap is intended to guide Trilogicon through a disciplined V1 build process.

The goal is not to rush features.  
The goal is to establish a reliable foundation.

## Current progress snapshot

- Phase 0 docs are in place (`vision`, `v1_scope`, `protocol_overview`, `design_principles`, `protocol_invariants`, `architecture`, `change_policies`).
- Phase 2 transaction validation baseline is implemented with tests:
  - deterministic unsigned payload hashing
  - Ed25519 signature verification
  - sender-to-public-key binding checks
- Phase 4 state transition baseline is implemented with tests:
  - valid transfer updates balances and sender nonce
  - stale nonce rejection without state mutation
  - insufficient balance rejection without state mutation
  - missing sender account rejection

Next primary target: Phase 3 block validation tests, then incremental chain-level validation hardening.

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
- the codebase is understandable enough to build V2 safely