# Protocol Invariants

These are the rules that should stay obvious in code and tests.

## Ledger

- Applying the same valid chain from the same genesis produces the same state.
- A transaction can only spend from the sender account.
- Sender nonce must match the expected nonce at application time.
- Fees are burned in V1/V2.
- Failed transaction application must not partially mutate state.

## Blocks

- A non-genesis block must point to the current parent hash for the live V1/V2 chain.
- Block height must increase by one from the parent.
- Block hash must match the canonical preimage.
- Transactions execute in block order.
- A block append is atomic: all selected transactions commit, or none do.

## Storage

- `chain.blocks` stores non-genesis blocks.
- Reloading from `chain.blocks` and the same genesis must reproduce the committed state.
- Corrupt or ambiguous storage fails closed.

## Network

- Peer metadata is advisory unless a versioned protocol doc says otherwise.
- Defensive limits may drop peers or messages, but must not redefine valid block bytes.

## V3 note

Future branch selection and reorg logic must preserve deterministic validity for any chosen canonical prefix. Height-first selection is an ordering rule, not a finality proof.
