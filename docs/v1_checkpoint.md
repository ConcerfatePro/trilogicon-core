# V1 checkpoint criteria

This checklist defines when the Trilogicon **V1 implementation** is considered complete for the current scope (`docs/v1_scope.md`). It is a project gate, not a security audit sign-off.

## Specification

- [x] `docs/v1_scope.md` matches shipped behavior; excluded features are not partially enforced in consensus paths.
- [x] `docs/modules.md` reflects the code layout (update when modules change).
- [x] Mempool policy is documented: submit-time checks vs seal-time state checks (`mempool.rs` module docs + this file).

## Automated tests (must pass)

- [x] `cargo test` in `node/` (all unit + integration tests) passes on **Linux, Windows, and macOS** in CI (`.github/workflows/ci.yml`); local development is often Linux-first.
- [x] Subprocess E2E: `cli_two_node_e2e` (merge genesis, send, matching chains).
- [x] Subprocess E2E: `cli_two_node_restart_e2e` (kill nodes, restart same data dirs, send again, chains still match).
- [x] Subprocess E2E: `cli_three_node_e2e` (producer gossips to two listeners; see test file for topology).

## Correctness coverage (ongoing)

- [x] Invalid transaction and block cases covered at library level (`transaction`, `block`, `blockchain`, `state`, `encoding`).
- [x] Named V1 matrix in `src/rejection_matrix_tests.rs` matches [`docs/protocol_invariants.md`](protocol_invariants.md#automated-rejection-coverage).
- [x] Network ingress path (`try_append_network_block`) covered for at least drift rejection vs `append_block`.

## Operations

- [x] `README.md` runbook matches current CLI flags and data-dir files.

## Honesty

- [x] README or `docs/v1_scope.md` states what V1 explicitly does **not** guarantee (e.g. production decentralization, partition behavior, DoS resistance).

When every item above is checked, the team may call **V1 implementation complete** and plan **V2** from that baseline using [`v2_scope.md`](v2_scope.md).
