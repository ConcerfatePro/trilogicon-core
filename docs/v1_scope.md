# Trilogicon v1 Scope

## Purpose

Trilogicon v1 is the first working version of the Trilogicon network.

Its purpose is to prove that the project can operate as a simple, secure, and understandable digital value-transfer system before adding broader features. V1 is intentionally narrow in scope. The goal is not to include everything at once. The goal is to make the core rules clear, reliable, and testable.

Trilogicon v1 should show that honest nodes can validate the same transactions and blocks, apply the same state transitions, and converge on the same chain state under the same rules.

## Chain identity

Trilogicon is being built as its own independent base-layer blockchain.

It is **not**:
- a token on another blockchain
- a smart contract deployed under another ecosystem
- a wrapped asset on another chain

Trilogicon has its own native asset, its own node software, and its own protocol rules.

## Native asset

The native asset of the network is:

- **Name:** Trilogicon
- **Ticker:** TRIL

In v1, TRIL exists as the single native asset used for:
- holding balances
- sending value between accounts
- paying transaction fees

V1 does not include multiple token types or token standards.

## Design priorities

Trilogicon v1 is designed around five priorities:

- **Simplicity**
- **Security**
- **Reliability**
- **Clarity**
- **Long-term usefulness**

These priorities matter more than feature count.

If a feature makes the system harder to understand, harder to validate, or harder to trust, it does not belong in v1.

## Ledger model

Trilogicon v1 uses an **account-based ledger model**.

Each account stores:

- **balance**
- **nonce**

This model was chosen because it is easier to reason about for the v1 goals and provides a clean foundation for signed value transfer.

### Balance

The balance is the amount of TRIL currently held by the account.

### Nonce

The nonce is the account’s transaction sequence value.

It is used to:
- enforce transaction ordering for each sender
- prevent replay attacks
- make state transitions more predictable

## V1 includes

Trilogicon v1 includes the minimum core components needed for a serious value-transfer blockchain.

### 1. Account creation and ownership

The network must support account creation and account ownership through cryptographic keypairs.

Users should be able to control an account through possession of the corresponding private key.

### 2. Signed TRIL transfers

The network must support sending and receiving TRIL through signed transactions.

A valid transaction should include core fields such as:
- sender
- receiver
- amount
- fee
- nonce
- timestamp
- signature
- transaction hash or identifier

The exact encoding must be deterministic and clearly defined by the protocol.

### 3. Signature verification

The node must verify that:
- the transaction was signed correctly
- the public key is valid
- the claimed sender is actually authorized to spend from that account

A valid signature alone is not enough if it does not correspond to the claimed sender account.

### 4. Replay protection

The network must reject replayed transactions.

This is primarily enforced through strict nonce validation.

A transaction must only be accepted if its nonce matches the sender account’s valid next sequence state under the protocol rules.

### 5. Balance enforcement

The node must reject transactions that try to spend more than the sender can afford.

At minimum, the sender must be able to cover:

`amount + fee`

Balances must never underflow.

### 6. Deterministic transaction validation

All honest nodes must evaluate transactions using the same rules and reach the same result when given the same transaction and the same state.

Validation must be deterministic.

That means Trilogicon v1 should avoid ambiguous serialization, hidden defaults, unclear ordering behavior, or logic that depends on local implementation quirks.

### 7. Block creation

The network must support grouping transactions into blocks.

A block contains protocol-defined data:
- block height
- previous block hash
- timestamp
- ordered transaction list
- block hash (must match the canonical header preimage)

V1 blocks do **not** carry a separate in-header producer identity or consensus proof field; sealing is handled by the node implementation under `docs/protocol_overview.md`.

### 8. Block validation

Nodes must validate blocks before accepting them.

At minimum, block validation should confirm:
- previous hash linkage is correct
- block height is correct
- block structure is well-formed
- included transactions are valid
- transactions are applied in order
- resulting state transitions are legal

### 9. Blockchain validation

Nodes must maintain a valid chain history and reject invalid chain data.

The chain should represent an ordered history of accepted blocks from which the current state can be derived.

### 10. Node synchronization

Multiple nodes must be able to synchronize with each other and converge on the same valid chain and state.

This is one of the most important practical goals of v1.

A blockchain is not proven by one node accepting its own local data. It is proven when multiple honest nodes independently validate and agree on the same history and state.

### 11. Fee support

Transactions in v1 should include an explicit fee field.

At minimum, the fee exists to:
- help discourage spam
- support basic transaction selection policy
- make transfer accounting explicit

The exact fee destination and handling rules should be documented clearly in protocol behavior.

## V1 consensus posture

Trilogicon v1 should use a practical and clearly documented consensus baseline that supports:
- block production
- block validation
- chain comparison
- node synchronization

V1 does **not** need to solve every advanced consensus problem immediately.

The v1 consensus design should be realistic, explainable, and strong enough to support a functioning multi-node transfer network.

Stronger consensus and validator-model improvements belong in later versions once the base protocol is stable.

## Supply and genesis expectations

V1 should clearly define the network’s initial supply behavior and genesis state.

At minimum, the protocol should document:
- how the genesis state is created
- where initial balances come from
- whether supply is fixed at genesis or can change under explicit protocol rules
- whether any block reward exists in v1 or whether the network is fee-only

Even if these choices evolve later, v1 must still define them clearly enough that honest nodes can agree on valid state.

## V1 non-goals

The following are intentionally **out of scope** for Trilogicon v1:

- smart contracts
- DeFi systems
- bridges
- governance systems
- multiple token standards
- advanced staking
- complex validator economies
- privacy features
- NFTs
- application-layer ecosystems
- ultra-high TPS optimization
- hype-driven token mechanics
- unnecessary complexity added just to appear advanced

These are excluded because they would increase complexity before the core transfer system is fully proven.

## What success looks like

Trilogicon v1 is successful if it can do the following well:

- create accounts
- track balances correctly
- enforce nonces correctly
- verify signatures correctly
- reject invalid transactions and invalid blocks
- prevent replay attacks
- maintain deterministic state transitions
- synchronize across multiple nodes
- keep the protocol understandable and auditable

V1 is **not** successful merely because it has many features.

V1 is successful if the foundation is trustworthy.

## Versioning philosophy

Trilogicon is intended to grow through multiple versions.

### V1
Simple, secure value transfer.

### V2
**Node hardening** for the linear V1 protocol: peer/session safety, sync/catch-up, persistence/restart, local mempool hygiene, observability—**no** consensus or economics changes beyond [`v2_scope.md`](v2_scope.md) (protocol freeze).

### V3
Stronger consensus and performance improvements.

### V4+
Possible broader programmability or ecosystem features, but only if the core system is already strong.

## Final scope rule

For Trilogicon v1, the standard should be:

**If a feature makes the core network more trustworthy, it belongs in consideration.  
If it only makes the network look more advanced, it should wait.**
