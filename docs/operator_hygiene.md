# Operator and repository hygiene

## Secrets and local node data

- **Never commit `wallet.seed`.** It is a signing secret for the reference wallet. Treat accidental commits like credential leaks.
- **Never commit live node data directories** (`node/data-*`, `chain.blocks`, `pending_tx.tril`, `genesis_bind.toml`, lock files under a data dir, and similar). These files are produced by `node init` / `node run` and are machine-local.
- **If `wallet.seed` (or equivalent material) was ever pushed to a public remote, assume compromise** for that key material. Rotate to a fresh wallet and genesis for any shared testnet or public demo; do not reuse the exposed seed for funds or long-lived identities.

## Recommended workflow

- Keep runtime directories **outside** the repository tree, or under paths listed in the root `.gitignore` (for example `node/data-*`).
- Use `cargo run -- init --data-dir /path/outside/repo` for experiments.
- For reproducible tests, rely on **temporary directories** created by integration tests, not checked-in chain files.

## Historical note

An earlier milestone commit accidentally added `node/data-a/` and `node/data-b/` with sample runtime files. Those paths were removed from the **git index** so they are no longer tracked; **git history still contains the old blobs** until a maintainer optionally rewrites history. If you cloned before that cleanup, run `git pull` and confirm `git ls-files` does not list `wallet.seed` under `node/`.
