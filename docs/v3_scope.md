# V3 Scope

Status: active design/planning. V3 is not live.

The live V1/V2 node still has one committed tip. `node/src/v3/` contains inert scaffolding and tests for future branch/index/reorg work. It is not called from `append_block`, network ingest, storage, CLI, or live consensus behavior.

## What V3 is trying to define

- how retained competing branches are represented;
- how a preferred tip is selected;
- how a reorg would be planned and bounded;
- what storage must exist before side branches are real;
- how confirmation/finality language should stay honest.

## Design docs

- [`fork_choice.md`](fork_choice.md) - height-first selection and temporary hash tie-break.
- [`reorg_model.md`](reorg_model.md) - reorg boundaries, storage needs, and mempool notes.
- [`finality.md`](finality.md) - confirmation wording and limits.
- [`protocol_v3_chain_rules.md`](protocol_v3_chain_rules.md) - deterministic validity vs local ingress policy.
- [`v3_test_plan.md`](v3_test_plan.md) - tests required before implementation.
- [`v3_block_index_rebuild.md`](v3_block_index_rebuild.md) - V3-08a block-index rebuild spec.

## Compatibility intent

V3 planning does not by itself change transaction signatures, fee burn, block hash rules, genesis, or V2 persistence behavior. Any real protocol delta needs a written versioned decision.

## Integration gate

Do not wire V3 into live behavior until these are done and reviewed:

- deterministic `BlockIndex` rebuild from stored canonical data, with tests;
- validated candidate-tip discovery before branch selection;
- reorg plan validation and preflight policy;
- replay sandbox coverage with typed errors;
- side-branch storage design;
- mempool-after-reorg policy and tests;
- operator/finality wording aligned with the actual behavior.

V3-08a only documents the block-index rebuild plan. It does not implement it.

## Non-goals

No smart contracts, DeFi, staking, delegated consensus, bridges, NFTs, storage migration, network reorg gossip, or production finality claims in this phase.
