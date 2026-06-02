# V2 Freeze

After `v2.0.0`, V2 is frozen as a node-hardening release.

Allowed under V2 maintenance:

- bug fixes that preserve V1/V2 consensus behavior;
- clearer docs and diagnostics;
- tests for already-specified behavior;
- build or CI cleanup;
- V2.1 polish that stays local/wire-compatible.

Not allowed under V2:

- changing transaction or block validity;
- changing fee burn or nonce rules;
- adding fork choice, reorgs, or side-branch storage;
- adding chain ID to signed payloads;
- changing genesis semantics;
- adding staking, rewards, governance, smart contracts, or bridges.

Consensus changes need a new versioned scope.
