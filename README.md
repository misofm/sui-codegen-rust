# sui-codegen-rust

First-party Rust bindings generator for Sui Move packages (private for now; will be open-sourced).

- `crates/movegen-bytecode` — strict, wasm-clean reader for Move bytecode (format v6–v7).
- `crates/movegen` — `movegen.toml` → lock (ABIs + per-network deployments) → generated code.
- `crates/movegen-cli` — the `movegen` binary: `refresh`, `generate [--check]`.
- `crates/movegen-runtime` — what generated code builds on (wasm-clean): typed PTB arguments,
  deployments, struct tags, strict decoding.
- `crates/movegen-e2e` — bindings for the whole fixture corpus (every Miso package, plus the
  `sui`/`std` framework), compiled natively, under clippy and for wasm, and exercised by tests.

## Usage

```toml
# movegen.toml
reference_network = "mainnet"

[networks.mainnet]
graphql = "https://graphql.mainnet.sui.io/graphql"

[packages.record_shop]
mainnet = "0x…"                       # any id in the package's lineage

[generate]
output = "src/generated"
functions = ["record_shop::listing"]  # packages or modules that get call builders
```

```sh
movegen refresh          # fetch ABIs + deployments into movegen.lock/ (commit it)
movegen generate         # offline: movegen.lock/ → src/generated/ (commit it)
movegen generate --check # CI: fail if the generated tree is stale
```

```rust
#[rustfmt::skip]
mod generated;
use generated::record_shop::listing;

let d = Deployment::from_lock_json(include_str!("../movegen.lock/deployments/mainnet.json"))?;
let pricing = listing::fixed(&mut tx, &d, 100u64)?;
let record = listing::purchase(
    &mut tx, &d, vec![currency],
    ObjectRef::shared(listing_id, listing_version),   // &mut Listing
    ObjectRef::shared(pressing_id, pressing_version), // &mut Pressing
    payment,                                          // Arg<Balance>, a previous result
    pricing,                                          // Arg<Pricing>
)?;                                                   // Clock injected; returns Arg<Record>
```

Inputs are added in parameter order, pure values are never de-duplicated, and shared-object
mutability is merged across uses — `@mysten/sui`'s rules. (Byte parity is proven for Miso's
purchase flow by golden vectors in `misofm/core`; a general differential harness is next.) A value of the wrong
Move type, or a normal object where a `Receiving<T>` is expected, does not compile.

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
