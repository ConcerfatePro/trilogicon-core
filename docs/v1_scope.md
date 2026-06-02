# V1 Scope

V1 defines the minimal transfer chain.

## In scope

- account balances and nonces;
- Ed25519 signed transfers;
- transaction hashing and signature checks;
- deterministic state transitions;
- block hashing and block validation;
- shared genesis state;
- replay from persisted blocks;
- basic TCP sync for a linear chain.

## Rules that matter

- Sender nonce must match the committed account nonce.
- Sender must have `amount + fee`.
- Fees are burned.
- Transactions apply in block order.
- A block must extend the current tip in the live node.
- The same genesis plus the same valid block sequence must produce the same state.

## Out of scope

- fork choice and reorgs;
- staking, validators, rewards, fee routing;
- smart contracts;
- state snapshots;
- production network security;
- chain ID in signed payloads.

V1 is complete for this narrow scope. V2 hardens the node around it without changing these rules.
