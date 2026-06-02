# V2 Persistence and Restart Notes

V2 makes local disk behavior explicit. The node should either load a chain that matches deterministic replay, or stop with a clear error.

## Files

- `chain.blocks`: non-genesis blocks in append order.
- `genesis_bind.toml`: commitment binding for the data directory.
- `pending_tx.tril`: queued transactions from `send`.
- `.node.run.lock`: exclusive lock for `node run`.

## `chain.blocks`

New files use:

```text
TRILBC01
u32_be length
encoded block
u32_be crc32(encoded block)
...
```

Legacy files without the magic header still load as length-prefixed block frames.

Startup fails closed on CRC mismatch, truncated body/CRC, decode error, replay error, or genesis mismatch. The only automatic repair is a short 1-3 byte incomplete next length prefix after at least one complete frame.

## Pending transactions

`send` appends one frame under `.pending_tx.lock`. `run` drains the file into the mempool and rewrites only after every parsed transaction has been accounted for. Parse failure leaves the file intact.

## Restart expectation

Same genesis plus same `chain.blocks` should reload to the same height and state. If a process hits a write/sync error, it poisons its in-memory store and refuses more appends until restart.

## Operator rule

Do not reuse a data directory with a different genesis once it has chain history. Use a fresh directory or an intentional reset.
