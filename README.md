# sui-codegen-rust

First-party Rust bindings generator for Sui Move packages (private for now; will be open-sourced).

- `crates/movegen-bytecode` — strict, wasm-clean reader for Move bytecode (format v6–v7).

See [docs/PLAN.md](docs/PLAN.md) for the design and [docs/SPEC.md](docs/SPEC.md) for the
numbered rules every implementation detail is tested against.

## Performance

`movegen-bytecode` is zero-copy: identifiers, constants and type signatures borrow from the
bytecode; signatures stay in their pre-order encoding and are walked without allocating; code is
bounds-checked as it streams past and never stored. On the fixture corpus (358 modules, 675 KiB,
release build; `cd oracle && cargo test --release --test speed -- --ignored --nocapture`):

| | movegen-bytecode | Mysten `move-binary-format` |
|---|---|---|
| deserialize + bounds check | 5.1 ms | 12.8 ms |
| + resolved ABI (names, types) | 12.3 ms | 21.4 ms (normalized module) |

Correctness is pinned by the oracle: identical ABIs on every corpus module, and over 289,453
corrupted modules Mysten never rejects one we accept.

## Development

```sh
scripts/install-sui.sh        # pinned sui CLI into .tools/ (fixtures + oracles only)
scripts/fetch-sui-src.sh      # pinned Sui sources into .tools/ (oracle tests only)
cargo test --workspace
(cd oracle && cargo test)     # differential tests against Mysten's move-binary-format
```
