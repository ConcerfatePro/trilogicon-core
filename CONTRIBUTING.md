# Contributing to Trilogicon

## Project layout

- **`node/`** — Rust crate: library + `node` binary (`cargo` commands run from here unless noted).
- **`docs/`** — Protocol scope, invariants, genesis, module ownership, V1 checkpoint checklist.

## Quick checks (same as CI)

GitHub Actions runs the same three steps on **ubuntu-latest**, **windows-latest**, and **macos-latest** for pushes and PRs to `main`, plus a **cargo audit** job (see `.github/workflows/ci.yml`). Dependabot proposes **GitHub Actions** updates weekly (`.github/dependabot.yml`).

From the repository root, using **GNU Make** (Git Bash, WSL, macOS, Linux):

```bash
make ci
```

Or run the underlying Cargo commands from `node/`:

```bash
cd node
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Stable Rust with `rustfmt` and `clippy` is recorded in `rust-toolchain.toml` at the repo root (`rustup` will pick it up when you `cd node`).

Subprocess integration tests (`cli_*_e2e.rs`) take roughly **40+ seconds** because they start real `node` processes and use wall-clock delays.

## Hygiene

- Do **not** commit `wallet.seed`, `chain.blocks`, `pending_tx.tril`, or local `data-*` directories (see `.gitignore`).
- Prefer keeping **consensus and validation** in `node/src/` core modules, not only in `main.rs` (see `docs/modules.md`).

## Pull requests

- Keep changes focused and consistent with `docs/v1_scope.md`.
- If you add a new rejection rule or invariant, extend `docs/protocol_invariants.md` and, when practical, `src/rejection_matrix_tests.rs`.
