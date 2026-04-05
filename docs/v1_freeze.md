# Trilogicon V1 Freeze

## Status
V1 is now feature-frozen.

## Included in V1
- native TRIL transfers
- account balances
- nonces
- signature verification
- block creation
- block validation
- chain validation
- node sync
- shared genesis
- persistence/restart behavior

## Not changing during freeze
- ledger model
- transaction core fields
- block core structure
- nonce semantics
- genesis model
- validation rules
- network message behavior unless fixing a bug
- consensus behavior unless fixing a bug

## Allowed during freeze
- bug fixes
- test fixes/additions
- logging improvements
- docs improvements
- operator/runbook improvements
- cleanup/refactors that do not change protocol behavior

## Not allowed during freeze
- smart contracts
- staking
- governance
- new token systems
- changing V1 scope
- changing consensus-critical behavior unless required for a bug/security fix
