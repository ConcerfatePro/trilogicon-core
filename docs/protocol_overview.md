# Protocol Overview

Trilogicon V1 is an account-based transfer protocol.

Each account has:

- `balance: u64`
- `nonce: u64`

A transaction moves `TRIL` from one account to another. It is signed by the sender and must use the sender's next expected nonce.

## Transactions

A transaction contains sender, receiver, amount, fee, nonce, timestamp, public key, signature, and transaction hash.

Validation checks structure, hash, signature, sender binding, amount, fee, nonce, and balance. State application is deterministic: subtract `amount + fee`, increment sender nonce, and credit the receiver with `amount`.

In V1, fees are burned. They are not paid to a proposer or treasury.

## Blocks

A block contains:

- height;
- previous block hash;
- Unix timestamp;
- ordered transactions;
- block hash.

The block hash is computed from the canonical header preimage. Blocks have no proposer identity, consensus proof, or producer reward field in V1/V2.

## Chain validation

The live V1/V2 node accepts linear extension only:

- height must be `tip.height + 1`;
- `previous_hash` must match the current tip;
- parent-relative timestamp rules must pass;
- every transaction must apply in order;
- state updates commit atomically for the block.

## Genesis

The genesis block is fixed in code. The genesis state comes from `genesis.toml` allocations. Nodes in the same network must use the same genesis file and commitment.

## Not in V1/V2

- smart contracts;
- staking or validator economics;
- fork choice or reorg execution;
- state snapshots over the wire;
- production finality claims.
