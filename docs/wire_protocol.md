# Trilogicon TCP wire protocol (V1 / v2 handshake)

Transport: **TCP**. Each message is a **4-byte big-endian `u32` length** followed by **length bytes** of payload. Length must not exceed **`MAX_WIRE_FRAME_BYTES`** (`64 MiB`, see `node/src/network.rs`).

All multi-byte integers are **big-endian** unless noted. String fields on the wire are **UTF-8** with an explicit length prefix where described.

Untrusted peers: decoders enforce batch sizes, frame caps, and canonical encoding rules; nodes must not crash on random input.

## Framed payload layout

```
payload = opcode (u8) || body (opcode-specific)
```

## Opcodes

| Byte | Name | Direction | Body / semantics |
|------|------|-----------|------------------|
| `1` | `OP_TX` | Gossip | Canonical encoded **transaction** (`encoding::encode_transaction`). |
| `2` | `OP_BLOCK` | Gossip | Canonical encoded **block**. |
| `3` | `OP_GET_BLOCKS` | Pull | `start_height: u64_be`. Request blocks with `height >= start_height`. |
| `4` | `OP_BLOCKS` | Pull reply | `count: u32_be`, then `count` times: `len: u32_be` + `encode_block` bytes. `count <= MAX_BLOCKS_PER_WIRE_BATCH` (`4096`). |
| `5` | `OP_HELLO` | Handshake | See **HELLO body** below. |
| `6` | `OP_HELLO_OK` | Handshake | Same body as HELLO; echoes agreement on version / network / genesis. |
| `7` | `OP_REJECT` | Handshake | Optional short reason after a fixed prefix (see implementation). |
| `8` | `OP_REQUEST_PEERS` | Peer exchange | No body (`payload.len() == 1`). Reply with `OP_PEERS`. |
| `9` | `OP_PEERS` | Peer exchange | `n: u32_be`, then `n` times: `addr_len: u16_be` + UTF-8 `host:port`. Caps: `n <= MAX_PEERS_PER_WIRE_FRAME` (`64`), each addr `<= MAX_PEER_ADDR_WIRE_BYTES` (`512`). |
| `10` | `OP_BLOCK_INV` | Gossip | `height: u64_be`, `hash_len: u16_be`, `block_hash` UTF-8 (`hash_len <= MAX_TIP_HASH_WIRE_BYTES`). Announces a tip without sending the full block. |
| `11` | `OP_BLOCK_WANT` | Gossip | Same layout as `OP_BLOCK_INV`; asks peer to send `OP_BLOCK` for that height/hash. |
| `12` | `OP_INV_ACK` | Gossip | No extra body (`payload.len() == 1`). Decline, already have, or not serving. |

## HELLO body

```
wire_version: u16_be
network_id:    u32_be
genesis_commitment: [u8; 32]
tip_height:    u64_be
tip_hash_len:  u16_be
tip_hash:      UTF-8 bytes
```

`wire_version` is **`TRIL_WIRE_VERSION`** (currently `2`). Peers must match **version**, **`network_id`**, and **`genesis_commitment`** after handshake.

## v2 handshake behavior (configurable)

`WireRuntimeConfig` (see `node/src/network.rs`) controls:

- **Outbound** (`handshake_outbound`): send `OP_HELLO` before other opcodes on **new outbound** connections.
- **Inbound** (`require_handshake_inbound`): first frame must be `OP_HELLO`.
- **Inbound** (`allow_legacy_inbound`): if handshake is not required, allow legacy first frames (`GET_BLOCKS`, `OP_TX`, etc.).

CLI mirrors these flags (`--handshake`, `--require-handshake-inbound`, `--no-legacy-inbound`).

## Block inventory / fetch (`--announce-blocks`)

When **`announce_blocks`** is enabled, outbound gossip sends **`OP_BLOCK_INV`** first. The peer may respond with **`OP_BLOCK_WANT`** (same height/hash), after which the announcer sends **`OP_INV_ACK`** or **`OP_BLOCK`**, or **`OP_INV_ACK`** if it will not fetch.

If the announced height is **more than one** above the local tip, the node responds with **`OP_INV_ACK`** and logs `tril:block_inv:` (includes remote socket, announced height, block hash, and local tip when the TCP peer address is known).

## Peer exchange (`--exchange-peers`)

After a successful pull sync, the node may open another connection, send **`OP_REQUEST_PEERS`**, read **`OP_PEERS`**, and merge new addresses into `peer_book.toml` (only **new** keys are counted for diagnostics).

## Sync pull rounds and per-call block cap

`sync_from_peer` loops **`GET_BLOCKS`** until an **empty** `OP_BLOCKS` batch, **`MAX_SYNC_PULL_ROUNDS`** (`256`) non-empty rounds, or applying the next batch would exceed **`MAX_BLOCKS_APPLIED_PER_SYNC`** (`262_144`, operational memory/CPU bound per call). The combination of rounds and **`MAX_BLOCKS_PER_WIRE_BATCH`** (`4096`) could admit at most **`MAX_BLOCKS_APPLIED_PER_SYNC_WIRE_MAX`** (`1_048_576`) blocks without this stricter cap. Each `OP_BLOCKS` batch is still capped at **`MAX_BLOCKS_PER_WIRE_BATCH`**.

## Inbound session cap

Each accepted connection processes at most **`MAX_FRAMES_PER_INBOUND_SESSION`** (`8192`) framed messages, then stops with an error.

## References

- Canonical tx/block encoding: `node/src/encoding.rs`
- Implementation: `node/src/network.rs`
- Operator-facing overview: [`README.md`](../README.md) (networking flags)
