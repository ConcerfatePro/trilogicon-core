# Trilogicon — contributor shortcuts (run from repo root).
# On Windows without `make`, run the commands in CONTRIBUTING.md from `node/`.

.PHONY: fmt fmt-check clippy test ci

fmt:
	cd node && cargo fmt --all

fmt-check:
	cd node && cargo fmt --all -- --check

clippy:
	cd node && cargo clippy --all-targets -- -D warnings

test:
	cd node && cargo test

ci: fmt-check clippy test
