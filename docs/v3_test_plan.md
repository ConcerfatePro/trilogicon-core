# V3 Test Plan

Status: planning. Tests here gate future V3 implementation.

## Block index

- rebuild from empty/missing `chain.blocks` to genesis-only index;
- rebuild from valid legacy and V2-framed chain files;
- reject duplicate block hash;
- reject missing parent, height gap, parent mismatch, bad block hash;
- surface storage corruption distinctly from ancestry errors;
- prove cold replay and rebuilt index agree on canonical tip.

## Branch selection

- higher height wins;
- equal height uses deterministic hash tie-break;
- empty candidate list behavior is explicit;
- invalid or unknown tips never reach selection.

## Reorg plan and preflight

- valid rollback/apply suffixes pass;
- broken chains, duplicates, and bad fork placement fail;
- depth policy rejects before replay;
- no-op same-tip plan is handled.

## Replay sandbox

- cloned state is not mutated on failure;
- missing block material fails closed;
- timestamp, basic validation, index-link, and state errors are typed;
- forked rollback/apply scenarios produce expected final fingerprints.

## Storage and mempool before live integration

- side-branch storage survives restart or fails closed;
- canonical tip metadata and state agree after crash scenarios;
- mempool is revalidated after reorg;
- old-suffix transactions are handled by documented policy.

No V3 implementation should enter live commit/network paths without tests tied to these cases.
