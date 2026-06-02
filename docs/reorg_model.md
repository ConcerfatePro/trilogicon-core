# V3 Reorg Model

Status: design note. The live node does not execute reorgs.

A reorg would move the committed chain from one valid retained tip to another: find the fork point, roll back the old suffix, apply the new suffix, then update local storage and mempool state.

## Terms

- **Committed tip:** block currently reflected in local state.
- **Fork point:** last common block between old and new branches.
- **Old suffix:** blocks removed from the old canonical chain.
- **New suffix:** blocks applied from the selected branch.

## `MAX_REORG_DEPTH`

`MAX_REORG_DEPTH` is intended as local fail-closed policy for automated reorg execution. It is not deterministic block validity and not an input to abstract fork choice.

If the old suffix is too deep, the node should stay on its current committed tip and report the reason.

## Storage requirements before implementation

A future implementation needs:

- blocks addressable by `block_hash`;
- parent index (`block_hash -> parent_hash`, height);
- durable canonical tip metadata;
- crash recovery that does not serve state inconsistent with the recorded tip;
- a migration plan from V2 `chain.blocks`.

V2 storage is only a linear canonical log. It is not enough for side branches.

## Mempool after reorg

After a successful reorg, queued transactions must be rechecked against the new state. Invalid or stale transactions are dropped. Old-suffix transactions are not automatically requeued unless an explicit local policy is added and tested.

## Open work

- side-branch disk format and retention limits;
- rollback/apply implementation;
- crash recovery around canonical-tip changes;
- operator messages for refused deep reorgs;
- tests tying storage, state replay, and mempool cleanup together.
