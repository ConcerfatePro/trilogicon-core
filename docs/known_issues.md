# Known Issues and Limits

Trilogicon is a small reference chain. These are expected limits, not all bugs.

## Protocol and consensus

- No live fork choice or reorg execution in V1/V2.
- No production finality model.
- No staking, validator set, rewards, fee routing, governance, or slashing.
- No smart contracts or multi-asset system.
- No chain ID in signed transaction payloads yet.

## Node behavior

- Sync is linear catch-up from the current tip. Competing branches are rejected by the live node.
- Mempool behavior is local policy. Nodes can have different pending queues.
- `--max-future-drift-secs` affects network block admission. Shared deployments should use the same value everywhere.
- Damaged `chain.blocks` requires operator repair or reset.

## Tooling

- The dev UI is localhost-only development tooling.
- Faucet/testnet payout tooling is separate from core consensus and should be treated as testnet infrastructure.
