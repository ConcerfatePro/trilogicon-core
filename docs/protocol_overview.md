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

A V1 block includes:

- block height
- previous block hash
- timestamp
- transaction list
- block producer / proposer data
- consensus-related proof or authority field
- block hash

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
- valid timestamp rules
- valid block structure
- valid transaction ordering
- valid transaction set
- valid block hash
- valid consensus/producer proof for the chosen V1 mechanism

### Chain validation should include:

- valid genesis starting point
- uninterrupted previous-hash links
- valid block sequence
- valid state transitions at every block
- no invalid replayed transactions accepted by state rules

---

## Replay protection

Replay protection is provided primarily through per-account nonces.

Each sender account must submit transactions with the next expected nonce.

A transaction using an old nonce must be rejected.  
A transaction skipping ahead past the expected nonce must also be rejected unless future mempool policy explicitly allows waiting transactions.

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

## Design priority summary

Trilogicon V1 prioritizes:

- correctness
- clarity
- deterministic behavior
- security-first validation
- extensibility without over-complication