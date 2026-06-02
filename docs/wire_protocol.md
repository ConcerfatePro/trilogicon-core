# Wire Protocol

The reference node uses TCP with length-prefixed frames.

Each message is:

```text
u32_be length
payload bytes
```

Frames over the configured maximum are rejected before allocation-heavy work.

## Session

V2 peers exchange a small handshake before normal messages:

- wire version;
- genesis state commitment;
- advisory height.

A genesis commitment mismatch disconnects the peer. It does not change local block validity.

## Application messages

The current wire protocol supports transaction gossip, block gossip, and linear block requests/responses used by sync.

Peer-reported height and tip data are advisory. V2 sync still only appends blocks that extend the node's current tip.

## Not present

- fork-choice gossip;
- branch inventories;
- reorg announcements;
- state snapshots;
- authenticated peer identity.
