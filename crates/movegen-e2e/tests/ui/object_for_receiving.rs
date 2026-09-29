// A normal object reference cannot be passed where `Receiving<T>` is expected.
use movegen_e2e::generated::sui;
use movegen_runtime::{Deployment, ObjectRef};
use sui_transaction_builder::TransactionBuilder;

fn build(tx: &mut TransactionBuilder, d: &Deployment, registry_currency: ObjectRef) {
    let _ = sui::coin_registry::finalize_registration(tx, d, vec![], registry_currency);
}

fn main() {}
