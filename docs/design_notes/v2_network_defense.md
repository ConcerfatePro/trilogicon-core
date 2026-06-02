# V2 Network Defense Notes

V2 bounds peer work. A peer should not be able to force unlimited decode, sync, or append attempts.

## Rules

- Handshake first: wire version and genesis commitment must match.
- Frame sizes are capped before allocation-heavy work.
- Idle/read/write timeouts are local policy.
- Stale block spam, invalid decodable blocks, and inbound transactions have per-session budgets.
- Peer errors are typed where possible; disconnect logic should not depend on string matching.

## What this does not do

- It does not add authenticated peers.
- It does not add fork choice or branch repair.
- It does not redefine valid blocks or transactions.

Accepted blocks still go through the normal network append path.
