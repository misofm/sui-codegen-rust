// A receiving reference cannot be passed as a normal object.
use movegen_e2e::generated::record_shop;
use movegen_runtime::{Deployment, ReceivingRef};
use sui_transaction_builder::TransactionBuilder;

fn build(tx: &mut TransactionBuilder, d: &Deployment, listing: ReceivingRef) {
    let _ = record_shop::listing::price(tx, d, vec![], listing);
}

fn main() {}
