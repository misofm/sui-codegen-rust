# misofm/sui-codegen-rust — plan (v2, after adversarial review)

Goal: a first-party, best-in-class generator of Rust bindings for Sui Move packages. Private
now, open source later. Priorities: correctness (spec + test driven) > DX > performance.

Changes from v1 are marked **[v2]**. Review findings referenced as C1, H1–H5, M1–M6, L.

## 1. Principles

1. **On-chain bytecode is truth.** ABIs come from package BCS (`sui_sdk_types::MovePackage`:
   module bytes + type origins + linkage) decoded by **our own Move bytecode reader** [v2 H4].
   The same reader handles local `sui move build` output, so unpublished packages work.
   Parameter names/docs come from `sui move summary` when sources exist; a mismatch drops them
   with a warning, never fails [v2 M6].
2. **Offline, deterministic generation from a committed lock**; only `refresh` touches the
   network; `generate --check` gates CI.
3. **Network data is runtime data.** Package ids, type origins, and system-object initial shared
   versions live in per-network deployment files and a generated typed `Deployment` struct
   [v2 H2, M3]. Only 0x1/0x2/0x3 addresses and Clock (0x6, v1) / SuiSystemState (0x5, v1) are
   constants.
4. **Type arguments are values.** Calls, type tags and event decoding take `TypeTag`s at
   runtime; Rust generics are optional sugar via `MoveType::type_tag(&Deployment)` [v2 C1].
5. **Parity target, stated precisely:** for the same sequence of operations, generated builders
   produce the same PTB as @mysten/sui `Transaction` semantics (argument materialization in
   parameter order, object dedup by id with OR-merged mutability, pure never deduplicated,
   system-object injection per @mysten/codegen `normalizeMoveArguments`) [v2 H1].
6. **Generated code = data + thin typed wrappers over one small runtime.** Hand-emitted trait
   impls (prost-style), no proc macros, no std derives that bound phantom params [v2 L].
   Native + wasm32 (no_std dropped [v2 L]).
7. **Never silently drop or guess**: structured diagnostics; unsupported items emit a visible
   stub comment and a warning.

## 2. Naming [v2 L]
Repo `misofm/sui-codegen-rust` (as requested). Crates use a neutral prefix so nothing implies
Mysten ownership: `movegen` (library), `movegen-cli` (binary `movegen`), `movegen-runtime`,
`movegen-bytecode` (reader). Final names confirmed before open-sourcing.

## 3. Architecture

```
refresh:  GraphQL packageBcs (per network, pinned checkpoint) ──┐
          `sui move build` output (local, unpublished) ──────────┼→ movegen-bytecode → lock/
          `sui move summary` (names/docs, optional) ─────────────┘
generate: lock/ → IR (resolve, symbolize lineage) → passes → RustIR → quote+prettyplease → crate
```

### 3.1 Lock (`movegen.lock/`) [v2 H3, H4]
- `abi/<network>/<package>@v<version>.json`: normalized ABI per network, per package version,
  with addresses symbolized by a **lineage-aware symbol table** (original id, every storage id,
  every defining id → `@package`). Includes constant pools (clever errors later).
- `deployments/<network>.json`: per package `original_id`, `published_at`, `version`,
  `type_origins`, `linkage`; system objects with initial shared versions (0x8, 0x403, 0xacc,
  plus opt-in 0xc/0xd/0xa).
- Generation uses a **reference network** (config; default mainnet). Other networks are checked
  for **upgrade compatibility** against it (identical datatype layouts, function superset,
  identical signatures for shared functions); the generator emits per-network availability
  metadata. System-package drift → warning; user-package incompatibility → error with diff.
- Header: lock schema version, movegen version, network, checkpoint, node version.

### 3.2 movegen-bytecode (in-house reader) [v2 H4]
Parses Move bytecode format v6–v7 tables needed for ABIs: module handles, identifiers, address
identifiers, datatype handles/defs (structs, enums incl. variants), field defs, function
handles/defs (visibility, `entry`, `macro` absent on chain), signatures, constant pool; skips
code units. Strict: bounds-checked, no panics, fuzzable, wasm-clean. Oracles: Mysten's
`move-binary-format` normalized modules (git dep at `mainnet-v1.80.1`, dev/test only) and
`sui move summary --bytecode` output over a corpus (0x1/0x2/0x3, DeepBook, all Miso packages).

### 3.3 IR and passes
Keyed by `(PackageSym, Module, Name)`. Passes (each insta-snapshotted):
1. **Resolve & canonicalize** — mirror `sui-package-resolver`: linkage-relative relocation,
   defining ids via type origins, arity checks, depth/width/node limits.
2. **Closure** — reachable datatypes from requested packages only.
3. **Classify parameters** into `ArgKind` [v2 M1]:
   `Pure(PureLayout)` (bool, u8–u256, address, `String`, `ascii::String`, `ID`, `Option<pure>`,
   `vector<pure>`), `Object{Imm|Mut|ByValue}`, `Receiving`, `System(obj, mutability)`,
   `TxContext{Imm|Mut}` (omitted; by-value → not callable), `Value{ByValue|Imm|Mut}` (non-pure
   non-object, must be a command result), `Generic{idx, ref}` (resolved at call time from the
   type argument: pure vs object). `Url`/`TypeName` are NOT pure (tested).
4. **Callability** — callable iff `public` or `entry` (any visibility) [v2 M2]; reference
   returns → target + signature only; by-value TxContext → not callable.
5. **Naming** — pure function of Move FQN + overrides; `r#` keywords; `self`/`Self`/`crate`/
   `super` → `_` suffix; module-scoped deterministic suffixes; `__` internals; absolute paths.
6. **Lowering** — one file per Move module, one module per package, Cargo feature per package.

### 3.4 Runtime (`movegen-runtime`)
Depends on `sui-sdk-types` 0.4 (serde, hash), `sui-transaction-builder` 0.4
(`default-features = false`), `serde`, `bcs` (align with sui-sdk-types' 0.1 to avoid two
copies in wasm — measure). Provides:
- `MoveType` (`fn type_tag(&self_or_ctx, &Deployment) -> TypeTag`), `Dyn(TypeTag)` for runtime
  type args; `MoveStruct` consts (package sym, module, name, abilities, type params).
- Framework mappings [v2 M5]: `UID`/`ID` → `Address` newtypes, `String`, `ascii::String`,
  `Option<T>`, `Balance<T>`, `Coin<T>`, `Url`, `TypeName`, `sui_sdk_types::U256` [v2 L],
  `VecMap`, `VecSet`, `Table`/`Bag` headers. Other framework types generated on demand.
- Strict decoding: tag-checked (`StructTag` incl. type args' defining ids), trailing-byte
  rejection, depth limits (mirror move-core-types annotated visitor rules).
- Typed `Arg<T>` handles (carry the builder `Argument`; object args also carry object id) and
  `IntoCallArg<T>`: an existing `Arg`, a lazy pure value, or a lazy object ref — materialized
  **inside the generated call in parameter order** [v2 H1]. Shared objects are created with
  `mutable: None`; each call re-applies mutability from its signature through the builder's
  merge (reproduces TS OR-merge). System objects injected at their parameter position with TS
  rules: Clock immutable v1; Random immutable; DenyList, 0x5, 0xacc mutability from signature;
  versions from `Deployment` [v2 H2].
- Results: single return → `Result(i)`; multiple → tuple of `NestedResult(i, j)` [v2 L].
- `MoveTarget` (package, module, function, type args) with `matches(&MoveCall)`; static
  `SIGNATURE: &[ArgKind]` for inspectors. Inspector matching is exact `published_at` by default,
  lineage-aware opt-in [v2 M3].

### 3.5 Generated API (target shape)
```rust
pub struct Listing { pub id: Uid, pub release_id: Id, pub pressing_id: Id,
                     pub pricing: Pricing, pub state: State, pub total_proceeds: u128 }
// phantom Currency is not a field; the type argument lives in the tag:
impl Listing { pub fn struct_tag(d: &Deployment, currency: TypeTag) -> StructTag;
               pub fn decode(d: &Deployment, tag: &StructTag, bcs: &[u8]) -> Result<Typed<Self>>; }
pub enum Pricing { Fixed(u64), Floor(u64) }

pub fn purchase(tx: &mut TransactionBuilder, d: &Deployment, currency: impl Into<TypeArg>,
                listing: impl IntoCallArg<Listing>, pressing: impl IntoCallArg<Pressing>,
                payment: impl IntoCallArg<Balance>, expected_pricing: impl IntoCallArg<Pricing>)
                -> Result<Arg<Record>, CallError>          // Clock injected at its position
pub mod purchase { pub fn target(d: &Deployment, currency: TypeTag) -> MoveTarget;
                   pub const SIGNATURE: &[ArgKind] = &[...]; }
// generic events: EventEnvelope { type_args: Vec<TypeTag>, value: T }
```
(Exact phantom representation is decided by a spike in M2: dropping phantom params from Rust
types and carrying type args in `Typed<T>` vs `PhantomData`. Decision recorded in SPEC.)

`Deployment` is a generated struct per output crate (fields per package + system objects), so
completeness is checked at compile time; loaded from `deployments/<network>.json` or built by hand.

## 4. Correctness strategy
`SPEC.md` has numbered rules; every rule is referenced by ≥1 test (CI script, M5).
1. **Unit + insta snapshots** per pass and of rendered code.
2. **Compiled goldens** for fixtures and corpus: native + wasm32, clippy `-D warnings`, rustdoc.
3. **Bytecode reader oracles**: move-binary-format (git, test-only) and `sui move summary
   --bytecode`, over 0x1/0x2/0x3, DeepBook (upgrade corpus: 5 defining ids), all Miso packages.
4. **Layout oracle**: `sui-package-resolver` (git, test-only, native) — our IR layouts and
   canonical type tags equal the resolver's for every datatype in the corpus, incl. upgraded
   DeepBook types and generic instantiations.
5. **BCS round-trip on recorded chain data**: committed corpus of objects/events (node `bcs` +
   `json`) — decode, re-encode byte-identically, check variant names/keys. A JSON projection
   ported from `RpcVisitor` (pinned tag) is an additional check, not the primary one [v2 M4].
6. **PTB parity**: scripted operation sequences run through @mysten/sui `Transaction` (TS
   harness, offline, objects pre-resolved like core's golden generator) vs generated Rust
   builders → byte-identical `TransactionKind`; includes system-object positions, repeated
   objects with mixed mutability, pure duplicates, multi-returns.
7. **Property tests**: proptest round-trips; determinism (shuffled lock, two runs → identical);
   naming stability.
8. **trybuild compile-fail**: wrong arg type, `Arg<Coin>` vs `Arg<Balance>`, reference-returning
   function builder, etc.
9. **Fixtures** (pinned `sui` 1.80.1 binary in `.tools/`, script-installed): phantom enums,
   positional, empty struct, generic constraints, hot potato, Receiving, multi/reference
   returns, private entry, FRIEND+entry, TxContext `&`/`&mut`, generic params. Upgrade semantics
   covered by the DeepBook mainnet corpus (localnet upgrade fixture deferred).
10. **Integration**: misofm/core purchase recipe on generated bindings passes all golden vectors.

## 5. Milestones [v2 H5]
- **M0** Repo, workspace, toolchain 1.97.1, CI (fmt, clippy, test, wasm32), SPEC skeleton,
  `.tools/` pinned sui CLI, oracle harness crate (git deps at `mainnet-v1.80.1`, not in default
  workspace build).
- **M1** `movegen-bytecode` + oracle tests; `refresh` from GraphQL `packageBcs` into the lock
  (Miso packages + deps, mainnet + testnet); deployments incl. system objects.
- **M2** IR, resolve/closure/classify/callability/naming passes; runtime types + strict decode;
  types renderer; phantom-representation spike; layout oracle + BCS round-trip corpus.
- **M3** Calls renderer, `IntoCallArg`/lazy materialization/mutability merge/system injection,
  targets + signatures, events; PTB parity harness; trybuild.
- **M4 — vertical slice**: generate bindings for Miso record/record_shop (+ needed framework),
  port `misofm/core` purchase build + inspect + `derive_sale_ids` onto them; all 10 build and
  51 inspect golden vectors pass; wasm size delta vs 191 KB brotli recorded.
- **M5** Breadth + polish: summaries merge (names/docs), `--check`/`explain`/`diff`, miette
  diagnostics, clever errors, framework crate with feature graph, perf budgets, SPEC coverage
  gate, fuzzing.

## 6. Deferred / cut
GraphQL-descriptor and gRPC GetPackage frontends (cut), no_std (cut), localnet upgrade fixture,
fuzzing, zero-alloc `matches`, clever errors, explain/diff, framework crate — all M5+.

## 7. Decisions after the second review pass (v3)

1. **Own bytecode reader (kept, deliberately).** The reviewer recommended a pinned git dependency
   on Mysten's `move-binary-format` for v1. We keep `movegen-bytecode` because (a) owning Sui
   tooling is an explicit goal, (b) a git dependency blocks publishing to crates.io, and (c) only
   the ABI tables are needed (not a VM loader), which keeps it small. Mysten's crate is the oracle
   (`oracle/`, path deps on a pinned sparse checkout), not a dependency.
2. **Milestone order:** M1 = reader + lock refresh; **M2 = vertical slice** (naive lowering of
   record/record_shop/balance, misofm/core's 10 build goldens + wasm size delta); M3 = hardened
   passes/types; M4 = calls/parity harness; M5 = breadth.
3. **Object handles:** kind is always explicit (`as_owned`/`as_shared`/receiving); lazy shared
   handles start with `mutable: None` and are OR-merged per use (by-value and non-MoveCall uses —
   TransferObjects/SplitCoins/MergeCoins/MakeMoveVec — count as mutable, as in TS). Handles with
   explicit mutability are never upgraded; passing an explicitly immutable handle where `&mut` or
   by-value is required is a `CallError`.
4. **Type tags from users** are normalized to defining ids (helper using lock type origins).
5. **Cross-network compatibility** checks public functions only (entry/friend may change);
   generation uses the most-upgraded compatible version of each package and emits per-network
   availability.
6. **Generic parameters:** the `IntoCallArg` variant (lazy pure, lazy object, existing `Arg`)
   decides pure vs object; pure bytes validated only when the tag is a pure layout.
7. **`Deployment` is trait-based** (`SystemObjects`, `HasPackage<P>`) so generated crates compose.
8. **Phantom params (option A):** dropped from Rust types; type args carried at runtime
   (`Typed<T>`, `EventEnvelope`, `Arg<T>` carries `Option<TypeTag>`); generated calls check
   type-arg agreement and return `CallError` on mismatch.
9. **PTB parity harness** runs the real `tx.build({ client })` against an offline mock client
   (objects/functions served from the lock), plus a pre-resolved matrix for explicit handles.
10. Consider direct BCS encode/decode emission instead of serde (single bcs copy in wasm) — decide
    in M3 with measurements.
