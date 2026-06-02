# V3-08a: BlockIndex Rebuild

Status: design note only. No live code is wired to this.

A future V3 node needs a deterministic way to rebuild a `BlockIndex` from persisted chain data. Fork choice and reorg planning are unsafe if the node cannot reconstruct trusted `block_hash -> parent_hash, height` facts after restart.

## Current code

`node/src/v3/block_index.rs` is an in-memory map with hardened ancestry walks and fork-slice helpers. It is unit-tested in isolation. It is not loaded from `chain.blocks`, not connected to `Blockchain`, and not authoritative for live behavior.

## Inputs a future rebuild needs

- canonical blocks from `chain.blocks` in file order;
- each block's hash, parent hash, and height;
- configured genesis state and the fixed genesis block identity;
- existing storage decode/framing rules;
- enough block material to revalidate or replay where needed;
- later, side-branch blocks only after a side-branch storage format exists.

## Proposed rebuild

1. Load stored block frames with the same storage rules used by startup.
2. Verify the configured genesis binding.
3. Decode each block.
4. Check structure, block hash, height, parent hash, and parent timestamp relation.
5. Insert a synthetic index root for `Block::genesis().block_hash` (`GENESIS_HASH`) with `parent_hash == ""` and `height == 0`.
6. Insert stored blocks in file order.
7. Validate ancestry from the canonical tip.
8. Fail closed on corruption, duplicate hash, missing parent, bad hash, height mismatch, parent mismatch, invalid genesis, or malformed ancestry.

If no stored blocks exist, the result is a genesis-only index. The canonical tip is the synthetic genesis root (`GENESIS_HASH`, empty parent, height 0), and ancestry validation runs against that row.

## Failure categories to preserve

Future code should expose typed failures along these lines:

- `MissingGenesis`
- `GenesisMismatch`
- `DecodeFailure`
- `DuplicateBlockHash`
- `MissingParent`
- `HeightMismatch`
- `ParentHashMismatch`
- `InvalidBlockHash`
- `InvalidAncestry`
- `StorageCorruption`
- `UnsupportedSideBranchFormat`

## Relationships

`branch_select` must receive tips from a validated index, not arbitrary hashes.

`replay_sandbox` still needs full block bodies and parent material. A correct index is necessary but not enough.

V2 storage is canonical-chain oriented. Side branches require a separate storage design before live reorgs can exist.

## Non-goals

No live fork choice, reorg execution, side-branch storage, storage migration, network changes, mempool changes, CLI changes, or production finality claims.
