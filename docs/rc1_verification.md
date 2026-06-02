# V1 RC1 Verification

Historical RC1 checklist.

For the current tree, use:

```bash
cd node
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

Then run the local two-node smoke test from the README if you need manual verification.
