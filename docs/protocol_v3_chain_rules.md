# V3 Chain Rules Notes

Status: planning. No live V3 consensus path exists yet.

## Separation that matters

**Deterministic chain validity** is about block bytes, parent header, and parent state. It should be replayable from stored material.

**Local ingress policy** is about whether this node chooses to process or retain bytes now: wall-clock future drift, resource caps, peer limits, and similar checks.

Do not mix the two when implementing V3.

## Intended compatibility

Unless a future V3 delta says otherwise:

- transaction payload and signature rules stay V1/V2-compatible;
- fee burn remains unchanged;
- block structure and hash preimage stay unchanged;
- genesis state rules stay unchanged;
- V2 local persistence remains the migration source for canonical history.

## Open questions

- exact side-branch retention format;
- which local ingress checks, if any, replay sandbox should mirror;
- how operators configure automated reorg depth;
- what metadata future storage records for canonical tip and retained tips.
