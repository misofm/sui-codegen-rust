// Functions returning references have no builder (only `target`): PTBs cannot hold references.
use movegen_e2e::generated::sui;
use movegen_runtime::{Deployment, ObjectRef};
use sui_transaction_builder::TransactionBuilder;

fn build(tx: &mut TransactionBuilder, d: &Deployment, table: ObjectRef) {
    let _ = sui::table::borrow::target(d, vec![]); // fine
    let _ = sui::table::borrow(tx, d, vec![], table, 1u64);
}

fn main() {}
