# Trilogicon dev test UI (local only)

**Not a wallet. Not for production. Not for untrusted networks.**

Small Axum server on **`127.0.0.1`** that:

- Reads **`chain.blocks`** and **`genesis.toml`** from a `--data-dir` (read-only replay for display — does **not** apply the same tail-repair path as `node` startup, to avoid mutating disk from this tool).
- Shows pending **`pending_tx.tril`** frame count (parse-only).
- Detects whether **`.node.run.lock`** is held (likely `node run` active).
- **Queues transfers** like **`node send`** using **`wallet.seed`** in that data directory.
- **Reject-path helpers** (Send page → expandable section): optional **nonce override** (stale `n−1`, gap `n+1` presets) and **insufficient-balance** preset (`amount` = full balance + fee so `amount+fee > balance`). All still produce **valid signatures**; the running `node` rejects or drops them per protocol/mempool policy — use to exercise `[seal]` / `[mempool]` behavior locally.

## Run

From repository root:

```bash
cd dev-test-ui
cargo run -- --data-dir ../node/data-a
```

Or with an explicit listen address (must stay on `127.0.0.1`):

```bash
cargo run -- --data-dir ../node/data-a --listen 127.0.0.1:9847
```

Open **http://127.0.0.1:9847/** in a browser.

In another terminal, run the real node so blocks seal and peers work:

```bash
cd ../node
cargo run -- run --data-dir ../node/data-a --listen 127.0.0.1:9333 --interval-secs 2
```

## Security

- The binary **refuses** to bind to non-loopback addresses.
- **No authentication** — intended only on your machine.
- **Private keys** stay in `wallet.seed` on disk; this UI only uses them when you click send (same trust model as CLI `send` for that data dir).

## CI

The main project CI targets the **`node`** crate. This crate is optional; run locally:

```bash
cd dev-test-ui
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

(There are no tests yet; clippy/fmt still apply.)
