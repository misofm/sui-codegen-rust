//! Misuse that must not compile (docs/SPEC.md §C): each case's expected diagnostics are in
//! `tests/ui/*.stderr`. Refresh them with `TRYBUILD=overwrite cargo test -p movegen-e2e --test compile_fail`.

// spec: C-TYPE-1, C-RECV-1, C-CALL-2
#[test]
fn misuse_does_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
