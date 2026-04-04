# Trilogicon V1 Known Issues

## Current status
This file tracks known limitations, unfinished areas, and important notes during the V1 freeze and release-candidate phase.

## Known limitations
- V1 is intentionally narrow and does not include smart contracts, governance, staking, bridges, or multiple token systems.
- Current operator flow assumes shared manual genesis setup across nodes.
- Current tooling is focused on a simple transfer-first workflow, not advanced network operations.
- Fork-choice / reorg tooling is intentionally limited in the current V1 operator flow.
- State sync is based on replaying blocks on top of shared genesis rather than advanced snapshot sync.

## Notes for release-candidate phase
- The focus during freeze is bug fixing, testing, documentation, and reliability improvements.
- New features should be deferred to post-V1 planning unless they are required to fix a real correctness or security issue.

## RC1 hardening targets
- verify repeatable fresh-clone setup from the README
- verify invalid transaction rejection paths stay clean
- verify restart and persistence behavior stays stable
- verify shared-genesis setup is easy to follow
- improve operator-facing troubleshooting notes if needed

## Next RC1 task
- add operator troubleshooting notes for common setup mistakes (existing wallet.seed, shared genesis mismatch, node started without genesis, send only queues tx)
