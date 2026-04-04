# Trilogicon V1 Scope

## Purpose of V1

Trilogicon V1 delivers a narrow, secure, understandable value-transfer network.

V1 is not feature-complete; it establishes a trustworthy protocol foundation.

The goal is to do a few important things well:

- create accounts
- hold balances
- send and receive TRIL
- verify signatures
- prevent replay attacks
- validate transactions and blocks
- synchronize nodes correctly

---

## Included in V1

### Ledger and accounts

- native TRIL currency
- account-based state model
- balance tracking
- nonce tracking
- deterministic account updates

### Transactions

- signed transfers
- transaction hashing
- validation logic
- replay protection through nonces
- basic fee field support

### Blocks and chain

- block structure
- previous-hash linking
- block hashing
- transaction inclusion
- block validation
- blockchain validation

### State management

- deterministic state transitions
- rejection of invalid state changes
- prevention of negative balances
- nonce correctness enforcement

### Node behavior

- block creation
- block acceptance / rejection
- chain verification
- basic synchronization between nodes
- basic networking for block and transaction exchange

### Development and testing

- local multi-node testing
- unit tests for core protocol rules
- integration tests for chain/state behavior

---

## Excluded from V1

The following are out of scope for V1:

- smart contracts
- DeFi features
- staking systems
- governance systems
- bridges
- privacy layers
- multiple token types
- NFTs
- advanced validator economics
- advanced slashing systems
- complex on-chain programmability
- high-TPS optimization claims
- large-scale production infrastructure assumptions

These can be revisited in later versions only if they improve user value without weakening clarity or safety.

---

## Why V1 is intentionally narrow

V1 is intentionally narrow because:

- smaller scope reduces attack surface
- simpler rules are easier to verify
- correctness matters more than feature count
- testing becomes more realistic
- protocol behavior stays easier to understand
- future versions can grow from a stable base

A narrow V1 is a deliberate design decision.

---

## Minimum success criteria for V1

V1 is successful when all of the following are true:

- accounts can be represented and tracked correctly
- balances update correctly after valid transactions
- invalid transactions are rejected consistently
- replayed transactions fail because of nonce rules
- invalid signatures fail verification
- blocks with invalid contents are rejected
- the chain can be validated from genesis forward
- multiple nodes can reach the same valid chain state
- honest nodes applying the same chain reach the same final state
- the implementation is understandable enough to continue safely into V2

---

## V1 completion mindset

V1 completion does not mean "finished forever." It means:

- the protocol foundation is sound
- the rules are clear
- the implementation is stable enough to build on
- future changes can be made from a position of structure rather than confusion