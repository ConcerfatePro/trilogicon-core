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

## Carried forward as V2 backlog (see `docs/v2_scope.md`)

The following are **known V1 limitations** that map to **explicit V2** work items (node hardening, not new protocol features):

- **Pending queue vs seal loop:** queued **state-invalid** transactions (e.g. insufficient balance at seal time) can cause repeated `block production: insufficient balance` log noise until the operator clears `pending_tx.tril` or the tx becomes valid. V2 should define **mempool/pending hygiene** (revalidation, bounds, drop semantics) under **local policy only** per [`v2_scope.md`](v2_scope.md).
- **Restart and persistence:** follow [`docs/design_notes/v2_persistence_restart.md`](design_notes/v2_persistence_restart.md) and [`docs/v2_scope.md`](v2_scope.md#project-decisions-v2) (fail-closed chain load, pending file semantics, hard genesis/binding refusal).

## Next RC1 task
- add operator troubleshooting notes for common setup mistakes (existing wallet.seed, shared genesis mismatch, node started without genesis, send only queues tx)
