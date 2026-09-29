# Specification

Numbered rules. Every rule is referenced by at least one test (`// spec: B-HDR-1`), and the
reference implementation for each rule is cited. Sources are pinned to Sui `mainnet-v1.80.1`
(commit `782577ca40c9`), `external-crates/move/crates/move-binary-format/src/deserializer.rs`
(abbreviated `deser.rs`).

## B — Move bytecode reader (`movegen-bytecode`)

Scope: the tables needed to describe a module's ABI (types, functions, constants). Code units are
decoded only to skip them. Supported bytecode versions: 6 and 7 (the on-chain range at protocol
137: `move_binary_format_version = 7`, `min_move_binary_format_version = 6`).

| Rule | Statement | Reference |
|---|---|---|
| B-HDR-1 | The binary starts with magic `A1 1C EB 0B`. The unpublishable magic `DE AD C0 DE` and anything else is rejected. | `file_format_common.rs` `BinaryConstants` |
| B-HDR-2 | The version is a little-endian u32. Values ≤ 6 are unflavored. For larger values the top byte is the flavor and the low 24 bits the version; version 7 requires flavor `0x05` (Sui). Accepted versions: 6 and 7. | `BinaryFlavor`, `VersionedBinary::initialize` |
| B-ULEB-1 | Every ULEB128 is canonical: no redundant trailing zero groups, no overflow of u64; each field has its own maximum (tables, indices, counts). | `read_uleb128_as_u64`, `load_*` |
| B-TBL-1 | The table count (ULEB ≤ 255) is followed by that many headers `kind: u8, offset: ULEB u32, size: ULEB u32`. Unknown kinds are rejected. | `read_table` |
| B-TBL-2 | Sorted by offset, tables are contiguous from 0, each non-empty, no kind appears twice, and the data region fits in the binary. | `check_tables` |
| B-TBL-3 | Each table is read with a cursor bounded to its own byte range; entries must exactly fill it. Enum-related tables require version 7. | `new_cursor`, `check_cursor_version_enum_compatible` |
| B-END-1 | After the table data comes the self module handle index (ULEB); no bytes may follow. | `deserialize_compiled_module` (`TRAILING_BYTES`) |
| B-ID-1 | Identifiers are ULEB-length-prefixed, valid UTF-8, and valid Move identifiers. | `load_identifiers`, `Identifier::from_utf8` |
| B-ADDR-1 | The address pool's size is a multiple of 32; entries are raw 32-byte addresses. | `load_address_identifiers` |
| B-SIG-1 | Signature tokens: bool, u8–u256, address, signer, vector, `&`, `&mut`, datatype, datatype instantiation (arity ≥ 1), type parameter; nesting depth ≤ 256. | `load_signature_token` |
| B-ABIL-1 | Ability sets are a ULEB ≤ 0xF with bits copy=1, drop=2, store=4, key=8. | `load_ability_set`, `Ability` |
| B-TP-1 | Datatype type parameters are `(constraints, is_phantom)` where is_phantom is a ULEB ≤ 1. Function type parameters are constraints only. | `load_struct_type_parameter`, `load_function_handles` |
| B-DEF-1 | Struct definitions carry a flag: `1` native (no fields), `2` declared (fields). Enum definitions carry flag `2` and 1–127 variants. Fields are `(name, type)`. | `load_struct_defs`, `load_enum_defs` |
| B-FN-1 | Function definitions: handle index, visibility byte (`0` private, `1` public, `3` friend), extra flags (`2` native, `4` entry, nothing else), an acquires list that must be empty (Sui deprecates global storage), then a code unit unless native. | `load_function_def` |
| B-CODE-1 | Code units are `locals: signature index`, `count: ULEB`, then `count` instructions whose operands are decoded per opcode, then (v7) jump tables. Global-storage opcodes are rejected. | `load_code_unit`, `load_code`, `load_jump_tables` |
| B-IDX-1 | Every index in every table and every code operand is bounds-checked eagerly, as `BoundsChecker` does: handles, signatures (incl. datatype arity), constants, field/variant handles and instantiations, locals (≤ 255 with parameters), branch and jump-table targets (jump tables have one branch per variant), and type-parameter ranges. A bad index is an error, never a panic. | `check_bounds.rs` |
| B-ABI-1 | Datatype names, function names, variant names (per enum) and field names (per struct/variant) are unique. Mysten's deserializer accepts duplicates (its verifier rejects them later) and its normalized view silently collapses them; bindings must never see them. | verifier `DuplicationChecker` |
| B-ABI-2 | Every struct, enum and function definition belongs to the module itself. | verifier |
| B-ABI-3 | Type-parameter indices in constants are rejected (constants have no type parameters). | verifier |
| B-PKG-1 | A package object is BCS `MovePackage { id, version, module_map: BTreeMap<String, Vec<u8>>, type_origin_table: Vec<TypeOrigin>, linkage_table: BTreeMap<Address, UpgradeInfo> }`. | `sui-types` `move_package.rs` L82–97 |
| B-ORACLE-1 | For every module in the corpus (0x1, 0x2, 0x3, DeepBook, all Miso packages and their linked dependencies, mainnet and testnet), the normalized view equals Mysten's `move-binary-format` output. | `oracle/` |
| B-ORACLE-2 | For systematically corrupted corpus modules (bit flips at strided positions, truncations, a trailing byte), with Mysten's loader in Sui's configuration (v6–v7, no extraneous bytes, no global storage): if both accept, the ABIs are identical; if we reject and Mysten accepts, the reason is B-ABI-1–3; Mysten never rejects a module we accept. Dense run: 289,453 mutants, 0 exceptions. | `oracle/` |

## L — lock and refresh (`movegen`)

| Rule | Statement |
|---|---|
| L-CLO-1 | The lock holds the configured packages and their full dependency closure: every linkage entry of every fetched package is fetched, one batched request per dependency level. Each package is read at the latest version of its lineage. |
| L-SYM-1 | ABIs are symbolized: every id in a package's lineage (original id, storage id, type-origin defining ids, linked versions) becomes `@name`. An address outside the closure is an error. |
| L-SYM-2 | Configured packages use their config name. Unnamed dependencies are `std`/`sui`/`sui_system`/`bridge`/`deepbook_v2` for system packages, otherwise `pkg_<first module name>` — identical on every network. Two packages with one name is an error asking for a config name. |
| L-DEP-1 | `deployments/<network>.json` records the chain identifier, checkpoint, and per package the original id, `published_at` (latest storage id), version, type origins (`module::Datatype` → defining id) and linkage (dependency symbol → linked storage id and version). |
| L-DEP-2 | Well-known shared objects (`sui_system_state` 0x5, `clock` 0x6, `random` 0x8, `address_alias_state` 0xa, `coin_registry` 0xc, `display_registry` 0xd, `deny_list` 0x403, `accumulator_root` 0xacc) are recorded with their per-network initial shared versions. |
| L-DET-1 | Lock files are deterministic, pretty-printed JSON with a trailing newline; writing an identical lock changes no files; files the lock no longer contains are removed. |
| L-DET-2 | Refreshing an up-to-date lock changes no files (the previous checkpoint is kept when nothing else changed). |
| L-NET-1 | Networks are compared on symbolized ABIs; differences are reported (per module/function) against the reference network. |

## Later sections (added as milestones land)

- **R** — resolution (mirrors `sui-package-resolver`)
- **A** — argument kinds and callability
- **N** — naming
- **T** — generated types and BCS
- **C** — generated calls and PTB parity with @mysten/sui
