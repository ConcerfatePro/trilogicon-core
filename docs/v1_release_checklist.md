# V1 Release Checklist

Before tagging V1:

```bash
cd node
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

Check:

- README run commands still work;
- genesis docs match CLI behavior;
- no committed data directories or wallet seeds;
- release notes say what V1 does **not** provide.
