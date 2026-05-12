# Contributing

Issues and pull requests are welcome at
<https://github.com/nktkt/pi>.

- See [`ROADMAP.md`](https://github.com/nktkt/pi/blob/main/ROADMAP.md)
  for what is targeted at upcoming 1.x and 2.0 releases. Unchecked items
  are good candidates for first contributions; open an issue with the
  milestone tag before you start.
- See [`CHANGELOG.md`](https://github.com/nktkt/pi/blob/main/CHANGELOG.md)
  for what shipped in each release.

Before opening a PR:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same checks on macOS and Linux against stable Rust and the
declared MSRV (`1.80`).
