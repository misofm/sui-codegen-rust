// A pure value cannot be passed for an object parameter, nor an object for a pure one.
use movegen_e2e::generated::record_shop;
use movegen_runtime::{Deployment, ObjectRef};
use sui_transaction_builder::TransactionBuilder;

fn build(tx: &mut TransactionBuilder, d: &Deployment, o: ObjectRef) {
    let _ = record_shop::listing::price(tx, d, vec![], 5u64);
    let _ = record_shop::listing::fixed(tx, d, o);
}

fn main() {}
