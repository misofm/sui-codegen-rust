// A previous result of one Move type cannot be passed for another, and non-object results do not
// silently become generic arguments (use `Arg::cast`).
use movegen_e2e::generated::{record_shop, sui};
use movegen_runtime::{Arg, Balance, Deployment};
use sui_transaction_builder::TransactionBuilder;

fn build(tx: &mut TransactionBuilder, d: &Deployment, balance: Arg<Balance>, amount: Arg<u64>) {
    let _ = record_shop::listing::purchase(tx, d, vec![], amount, amount, balance, amount);
    let _ = sui::transfer::public_transfer(tx, d, vec![], balance, amount);
}

fn main() {}
