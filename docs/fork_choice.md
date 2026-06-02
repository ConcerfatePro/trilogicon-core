# V3 Fork Choice

Status: planned V3 design. Not active in the live node.

## Rule

Among retained valid tips:

1. higher height wins;
2. if heights match, compare `block_hash` lexicographically as a temporary deterministic tie-break.

The tie-break is grindable. It is only a deterministic ordering rule, not a security property.

## Inputs

Fork choice must run only on tips known to come from a validated block index or equivalent retained-block set. It must not select from arbitrary hashes.

## Not solved here

- block validity;
- side-branch storage;
- reorg execution;
- network gossip for competing tips;
- production finality.
