# sui-codegen-rust

First-party Rust bindings generator for Sui Move packages (private for now; will be open-sourced).

- `crates/movegen-bytecode` — strict, wasm-clean reader for Move bytecode (format v6–v7).

See [docs/PLAN.md](docs/PLAN.md) for the design and [docs/SPEC.md](docs/SPEC.md) for the
numbered rules every implementation detail is tested against.

## Development

```sh
scripts/install-sui.sh        # pinned sui CLI into .tools/ (fixtures + oracles only)
scripts/fetch-sui-src.sh      # pinned Sui sources into .tools/ (oracle tests only)
cargo test --workspace
(cd oracle && cargo test)     # differential tests against Mysten's move-binary-format
```
