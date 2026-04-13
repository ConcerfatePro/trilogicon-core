# Trilogicon Protocol Overview

## Overview

Trilogicon is a base-layer blockchain network with a native asset called `TRIL`.

The V1 protocol uses an account-based ledger model. Each account stores a balance and a nonce. Transactions move TRIL from one account to another and must be signed by the sender.

The network is deterministic: honest nodes applying the same valid chain must reach the same final state.

---

## Core V1 responsibilities

The V1 protocol is responsible for:

- account creation and representation
- balance tracking
- signed value transfers
- nonce-based replay protection
- transaction validation
- block validation
- blockchain validation
- state transition execution
- multi-node synchronization

---

## Ledger model

Trilogicon V1 uses an account-based model.

Each account stores:

- `balance: u64`
- `nonce: u64`

### Why account-based?

This model was chosen because it is:

- simpler to reason about for V1
- easier to extend later
- easier to teach and document
- a better fit for the project’s narrow transfer-focused design

---

## Asset

Native asset: `TRIL`

In V1, TRIL is the only supported asset on the network.

There are no tokens, smart contract assets, or multi-asset systems in scope for V1.

---

## Transaction model

A transaction represents a signed request to move value from one account to another.

### Expected fields

A V1 transaction includes:

- sender address
- receiver address
- amount
- fee
- nonce
- timestamp
- public key or equivalent verification material
- signature
- transaction hash / transaction ID

### Transaction purpose

Transactions must support:

- ownership verification
- replay protection
- deterministic hashing
- validation before inclusion in a block

---

## Block model

A block groups valid transactions and extends the chain.

### Expected fields

A V1 block in the **current** implementation and encoding consists of:

- block height (`u64`)
- previous block hash (hex string)
- timestamp (`timestamp_unix`, seconds)
- ordered transaction list (each transaction passes `basic_validate`)
- block hash (hex string; must match the canonical hash of the header preimage)

There is **no** separate producer identity, proposer field, or consensus proof payload in the V1 block structure. Block production in the reference node is **local** (interval-driven sealing from the mempool); that is **not** represented as extra header fields. A **later protocol version** could add such fields; that would **not** be V1/V2 node hardening.

### Block purpose

Blocks must support:

- ordered transaction execution
- chain linking
- deterministic validation
- reproducible state transitions

---

## State transitions

State transitions are the core of the protocol.  
For each valid transaction, state updates must be deterministic.

Example transfer flow:

1. verify transaction structure
2. verify transaction signature
3. verify nonce matches sender account expectation
4. verify sender has enough balance for `amount + fee`
5. subtract value and fee from sender
6. increment sender nonce
7. add value to receiver
8. apply fee handling according to V1 rules (see below)
9. commit resulting state

All honest nodes must apply the same valid transaction in the same way.

### V1 fee rule (fee burn)

In V1, the `fee` is **burned**: it is deducted from the sender together with `amount`, but **no account’s balance is increased by the fee**. TRIL represented by collected fees is removed from the circulating supply tracked by account balances.

This keeps V1 minimal (no miner or protocol treasury payout yet). A future protocol version could redirect fees to a block proposer or fee pool instead; that would be an explicit consensus / state rule change.

**Not V2:** redirecting fees or changing fee economics is **out of V2**; it requires a **separately versioned** protocol scope, not node hardening (`docs/v2_scope.md`).

---

## Validation model

Validation must be strict and explicit.

### Transaction validation should include:

- valid structure
- valid field values
- valid signature
- correct nonce
- sufficient sender balance
- valid sender and receiver formatting
- nonzero / allowed amount rules
- deterministic hash consistency

### Block validation should include:

- correct previous hash
- correct height progression
- valid timestamp rules (including consensus checks against parent / local clock where applicable)
- valid block structure
- valid transaction ordering
- valid transaction set
- valid block hash (matches canonical preimage)

### Chain validation should include:

- valid genesis starting point
- uninterrupted previous-hash links
- valid block sequence
- valid state transitions at every block
- no invalid replayed transactions accepted by state rules

---

## Replay protection

Replay protection is provided primarily through per-account nonces.

Each sender account must apply transactions with the next expected nonce.

A transaction using an old nonce must be rejected during block application.  
A transaction skipping ahead past the expected nonce must also be rejected during block application, even if local mempool policy holds it for a later seal attempt.

**V2 local producer policy:** block validity still rejects skipped nonces inside a block. The reference mempool may keep future-nonce entries and skip currently non-executable entries during local seal candidate scan, but it must never include a transaction before its nonce is expected by simulated state, reorder selected transactions, or make an invalid transaction valid. Nonce sorting, remote-funding speculation, and any change to block validity remain outside V2 (`docs/v2_scope.md`).

This rule ensures that signed transactions cannot simply be replayed repeatedly against the same account state.

---

## Signatures and ownership

Ownership is proven through cryptographic signatures.

A valid transaction must be authorized by the sender’s private key or the protocol’s approved account-signing design.

V1 should prioritize:

- standard, well-understood signature schemes
- explicit verification logic
- minimal ambiguity in address derivation and validation rules

---

## Consensus note

The exact V1 consensus approach may remain provisional early on, but it must satisfy:

- blocks must be independently validatable by all nodes
- honest nodes must reject invalid blocks deterministically
- the design must be simple enough to implement safely in V1
- early development may use a controlled or simplified production model before later consensus upgrades

Consensus complexity must not destabilize the protocol.

**Not V2:** **consensus upgrades** beyond the **linear V1** extension model (fork choice, reorgs, new producer rules, and similar) are **out of V2** unless shipped under a **later protocol version** document (`docs/v2_scope.md`).

---

## Networking note

V1 networking should focus on correctness before sophistication.

Initial networking goals:

- node discovery or configured peers
- transaction propagation
- block propagation
- chain sync
- basic conflict handling
- reliable message handling

Networking does not need heavy optimization in the first iteration, but it must support a correct multi-node test environment.

---

## Storage note

Persistent storage can start simple in V1 and improve later.

What matters first is that the logical model is correct:

- chain data is stored consistently
- account state can be reconstructed or persisted reliably
- blocks and transactions can be validated against stored state

---

## Next version (planning)

Reliability, synchronization, persistence, and operational hardening for the **node** (same V1 linear protocol) are scoped in [`v2_scope.md`](v2_scope.md). Persistence and restart semantics are spelled out in [`design_notes/v2_persistence_restart.md`](design_notes/v2_persistence_restart.md). **V2 wire session + linear sync** (handshake, batch caps, catch-up loop) is documented in [`design_notes/v2_wire_peer_sync.md`](design_notes/v2_wire_peer_sync.md). Directional notes here about **fee destination**, **speculative mempool**, and **consensus upgrades** are **not V2** unless adopted under a **future protocol version**. **Storage** notes in this document mean **local persistence** may still evolve in V2 when it does **not** change block/tx validity (`v2_scope.md` classification and protocol freeze).

---

## Design priority summary

Trilogicon V1 prioritizes:

- correctness
- clarity
- deterministic behavior
- security-first validation
- extensibility without over-complication
