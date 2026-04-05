# Trilogicon AI Instructions

You are helping develop Trilogicon, a from-scratch cryptocurrency project.

## Core project goal

Trilogicon is intended to be a user-focused cryptocurrency network that prioritizes:

- simplicity
- security
- reliability
- clarity
- long-term usefulness

The goal is not hype, chain-copying, or overloading V1 with features.

## V1 goal

Trilogicon V1 should be a simple, secure digital value-transfer network.

V1 should focus on:

- creating accounts
- tracking balances
- sending and receiving TRIL
- verifying signatures
- preventing replay attacks with nonces
- validating transactions
- validating blocks
- validating chain history
- synchronizing nodes

## What V1 should not include

Do not add these to V1 unless explicitly requested and strongly justified:

- smart contracts
- DeFi
- bridges
- privacy features
- governance systems
- multiple asset types
- advanced staking
- high-TPS complexity
- feature creep for marketing reasons

## Design philosophy

Prefer:

- clear and practical architecture
- deterministic state transitions
- strict validation rules
- readable Rust code
- understandable module boundaries
- explicit tradeoff explanations
- secure and auditable solutions

Avoid:

- complexity for appearance
- overengineering
- copying large-chain design choices without justification
- pulling future-version features into V1 by default

## Technical direction

- base-layer blockchain
- native asset: TRIL
- account-based ledger
- account stores balance and nonce
- Rust for the core node

Likely modules:

- wallet.rs
- transaction.rs
- block.rs
- blockchain.rs
- state.rs
- crypto.rs
- network.rs
- consensus.rs
- config.rs
- errors.rs
- types.rs

## How to help

When assisting:

- think like a protocol designer and systems engineer
- keep recommendations realistic
- explain tradeoffs
- say clearly when something is a bad V1 idea
- break large tasks into milestones
- teach the logic behind generated code
- explain what each part does and why it exists
- preserve continuity with existing Trilogicon decisions

## Coding style expectations

When writing code:

- prioritize correctness over cleverness
- keep validation explicit
- separate protocol logic from networking where possible
- keep state transition logic deterministic
- include comments where they help learning
- prefer readable code over compressed code
- add tests for important protocol behavior

## Documentation expectations

When writing docs:

- be precise
- be practical
- align with the project's narrow V1 mission
- distinguish clearly between current version and future versions
- preserve the project's identity as a simple, predictable, security-first transfer network
