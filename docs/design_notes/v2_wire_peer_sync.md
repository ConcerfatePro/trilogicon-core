# V2 Wire and Peer Sync Notes

V2 sync remains linear.

## Handshake

Peers exchange:

- wire version;
- genesis state commitment;
- advisory height.

Wrong version or genesis commitment disconnects the peer.

## Sync

A node asks for blocks starting from its next height. Responses are capped and must form a linear batch. Blocks are appended only if they extend the local current tip.

Peer height is advisory. It helps decide whether to ask for blocks, but it does not select a chain.

## Not in V2

- competing tips;
- branch inventories;
- reorgs;
- state snapshots;
- fork-choice gossip.
