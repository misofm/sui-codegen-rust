//! Generated call builders, end to end: the PTB they produce (docs/SPEC.md §C, §N).

use movegen_e2e::generated::{record, record_shop, royalty_pool, std as move_std, sui};
use movegen_runtime::{
    Any, Arg, CallError, Deployment, MoveStruct as _, ObjectRef, Receiving, ReceivingRef,
    make_move_vec,
};
use sui_sdk_types::{
    Address, Argument, Command, Digest, Input, ProgrammableTransaction, SharedInput,
    TransactionKind, TypeTag,
};
use sui_transaction_builder::{ObjectInput, TransactionBuilder};

fn mainnet() -> Deployment {
    Deployment::from_lock_json(include_str!("../deployments/mainnet.json")).unwrap()
}

fn testnet() -> Deployment {
    Deployment::from_lock_json(include_str!("../deployments/testnet.json")).unwrap()
}

fn ptb(mut tx: TransactionBuilder) -> ProgrammableTransaction {
    tx.set_sender(Address::ZERO);
    tx.set_gas_price(1_000);
    tx.set_gas_budget(1);
    tx.add_gas_objects([ObjectInput::owned(Address::ZERO, 0, Digest::ZERO)]);
    match tx.try_build().unwrap().kind {
        TransactionKind::ProgrammableTransaction(p) => p,
        other => panic!("not a PTB: {other:?}"),
    }
}

fn addr(byte: u8) -> Address {
    Address::new([byte; 32])
}

fn usdc() -> TypeTag {
    "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC"
        .parse()
        .unwrap()
}

fn shared(id: Address, version: u64, mutable: bool) -> Input {
    Input::Shared(SharedInput::new(id, version, mutable))
}

fn pure<T: movegen_runtime::__private::serde::Serialize>(value: &T) -> Input {
    Input::Pure(bcs::to_bytes(value).unwrap())
}

fn call(command: &Command) -> &sui_sdk_types::MoveCall {
    match command {
        Command::MoveCall(call) => call,
        other => panic!("not a move call: {other:?}"),
    }
}

// spec: C-ORD-1, C-SYS-1, C-GEN-1, N-NET-1
#[test]
fn record_purchase() {
    let d = mainnet();
    let mut tx = TransactionBuilder::new();
    let payment = sui::balance::zero(&mut tx, &d, vec![usdc()]).unwrap();
    let pricing = record_shop::listing::fixed(&mut tx, &d, 100u64).unwrap();
    let record = record_shop::listing::purchase(
        &mut tx,
        &d,
        vec![usdc()],
        ObjectRef::shared(addr(1), 5),
        ObjectRef::shared(addr(2), 7),
        payment,
        pricing,
    )
    .unwrap();
    let record_tag = record::record::Record::struct_tag(&d, vec![]).unwrap();
    // `Arg<Record>` flows into the generic `T: key + store` parameter.
    sui::transfer::public_transfer(
        &mut tx,
        &d,
        vec![record_tag.clone().into()],
        record,
        addr(9),
    )
    .unwrap();
    let p = ptb(tx);

    let clock = d.system_object("clock").unwrap();
    assert_eq!(
        p.inputs,
        vec![
            pure(&100u64),
            shared(addr(1), 5, true),
            shared(addr(2), 7, true),
            shared(clock.id, clock.initial_shared_version, false),
            pure(&addr(9)),
        ],
        "inputs in parameter order; clock injected immutable"
    );
    assert_eq!(p.commands.len(), 4);
    let purchase = call(&p.commands[2]);
    assert_eq!(purchase.package, d.published_at("record_shop").unwrap());
    assert_eq!(
        (purchase.module.as_str(), purchase.function.as_str()),
        ("listing", "purchase")
    );
    assert_eq!(purchase.type_arguments, vec![usdc()]);
    assert_eq!(
        purchase.arguments,
        vec![
            Argument::Input(1),
            Argument::Input(2),
            Argument::Result(0),
            Argument::Result(1),
            Argument::Input(3)
        ]
    );
    assert!(
        record_shop::listing::purchase::target(&d, vec![usdc()])
            .unwrap()
            .matches(purchase)
    );
    assert_eq!(
        record_shop::listing::purchase::SIGNATURE.len(),
        purchase.arguments.len()
    );

    let transfer = call(&p.commands[3]);
    assert_eq!(transfer.package, d.published_at("sui").unwrap());
    assert_eq!(
        transfer.type_arguments,
        vec![TypeTag::Struct(Box::new(record_tag))]
    );
    assert_eq!(
        transfer.arguments,
        vec![Argument::Result(2), Argument::Input(4)]
    );
}

// spec: N-NET-1 (same code, per-network ids and system object versions)
#[test]
fn targets_follow_the_deployment() {
    let (main, test) = (mainnet(), testnet());
    let on = |d: &Deployment| {
        record_shop::listing::purchase::target(d, vec![usdc()])
            .unwrap()
            .package
    };
    assert_eq!(on(&main), main.published_at("record_shop").unwrap());
    assert_eq!(on(&test), test.published_at("record_shop").unwrap());
    assert_ne!(on(&main), on(&test));
    assert_ne!(
        main.system_object("random").unwrap().initial_shared_version,
        test.system_object("random").unwrap().initial_shared_version
    );
}

// spec: C-PURE-2 (pure values are never de-duplicated, like @mysten/sui)
#[test]
fn pure_values_are_not_deduplicated() {
    let d = mainnet();
    let mut tx = TransactionBuilder::new();
    record_shop::listing::fixed(&mut tx, &d, 100u64).unwrap();
    #[allow(clippy::needless_borrows_for_generic_args)] // `&T` is the case under test
    record_shop::listing::fixed(&mut tx, &d, &100u64).unwrap();
    assert_eq!(ptb(tx).inputs, vec![pure(&100u64), pure(&100u64)]);
}

// spec: C-OBJ-1 (one input per object; mutability OR-merged across uses)
#[test]
fn shared_mutability_is_merged() {
    let d = mainnet();
    let listing = ObjectRef::shared(addr(1), 5);
    let mut tx = TransactionBuilder::new();
    record_shop::listing::price(&mut tx, &d, vec![usdc()], listing).unwrap();
    assert_eq!(
        ptb(tx).inputs,
        vec![shared(addr(1), 5, false)],
        "`&Listing` alone is immutable"
    );

    let mut tx = TransactionBuilder::new();
    record_shop::listing::price(&mut tx, &d, vec![usdc()], listing).unwrap();
    let pricing = record_shop::listing::fixed(&mut tx, &d, 1u64).unwrap();
    record_shop::listing::set_price(
        &mut tx,
        &d,
        vec![usdc()],
        listing,
        ObjectRef::shared(addr(3), 1),
        pricing,
    )
    .unwrap();
    let p = ptb(tx);
    assert_eq!(
        p.inputs[0],
        shared(addr(1), 5, true),
        "a later `&mut` use upgrades the input"
    );
    assert_eq!(
        call(&p.commands[0]).arguments[0],
        call(&p.commands[2]).arguments[0]
    );
}

// spec: C-OBJ-2 (an explicitly immutable shared object refuses mutable use)
#[test]
fn explicit_immutability_is_enforced() {
    let d = mainnet();
    let frozen = ObjectRef::Shared {
        id: addr(1),
        initial_shared_version: 5,
        mutable: Some(false),
    };
    let mut tx = TransactionBuilder::new();
    record_shop::listing::price(&mut tx, &d, vec![usdc()], frozen).unwrap();
    let pricing = record_shop::listing::fixed(&mut tx, &d, 1u64).unwrap();
    let payment = sui::balance::zero(&mut tx, &d, vec![usdc()]).unwrap();
    let err = record_shop::listing::purchase(
        &mut tx,
        &d,
        vec![usdc()],
        frozen,
        ObjectRef::shared(addr(2), 7),
        payment,
        pricing,
    )
    .unwrap_err();
    assert_eq!(err, CallError::Immutable(0));
}

// spec: C-TA-1
#[test]
fn type_argument_count_is_checked() {
    let err = record_shop::listing::purchase::target(&mainnet(), vec![]).unwrap_err();
    assert_eq!(
        err,
        CallError::TypeArguments {
            function: "listing::purchase",
            expected: 1,
            got: 0
        }
    );
}

// spec: C-SYS-1 (singletons are injected with the parameter's mutability)
#[test]
fn system_objects_are_injected() {
    let d = mainnet();
    let mut tx = TransactionBuilder::new();
    sui::random::new_generator(&mut tx, &d).unwrap();
    let received = ReceivingRef {
        id: addr(4),
        version: 11,
        digest: Digest::ZERO,
    };
    let currency = TypeTag::Struct(Box::new(
        sui::coin_registry::Currency::struct_tag(&d, vec![usdc()]).unwrap(),
    ));
    sui::coin_registry::finalize_registration(&mut tx, &d, vec![currency], received).unwrap();
    let (random, registry) = (
        d.system_object("random").unwrap(),
        d.system_object("coin_registry").unwrap(),
    );
    assert_eq!(
        ptb(tx).inputs,
        vec![
            shared(random.id, random.initial_shared_version, false),
            shared(registry.id, registry.initial_shared_version, true),
            Input::Receiving(sui_sdk_types::ObjectReference::new(
                addr(4),
                11,
                Digest::ZERO
            )),
        ]
    );
}

// spec: C-GEN-1 (type parameters take objects, pure values, or results)
#[test]
fn generic_parameters_accept_any_value() {
    let d = mainnet();
    let owned = ObjectRef::Owned {
        id: addr(5),
        version: 3,
        digest: Digest::ZERO,
    };
    let mut tx = TransactionBuilder::new();
    sui::transfer::public_transfer(&mut tx, &d, vec![usdc()], owned, addr(9)).unwrap();
    let some: Arg<Option<Any>> =
        move_std::option::some(&mut tx, &d, vec![TypeTag::U64], 7u64).unwrap();
    move_std::option::destroy_some(&mut tx, &d, vec![TypeTag::U64], some).unwrap();
    let p = ptb(tx);
    assert_eq!(
        p.inputs,
        vec![
            Input::ImmutableOrOwned(sui_sdk_types::ObjectReference::new(
                addr(5),
                3,
                Digest::ZERO
            )),
            pure(&addr(9)),
            pure(&7u64),
        ]
    );
}

// spec: C-VEC-1, C-RECV-1
#[test]
fn vectors_of_receiving_references() {
    let d = mainnet();
    let coin = |b: u8| ReceivingRef {
        id: addr(b),
        version: u64::from(b),
        digest: Digest::ZERO,
    };
    let mut tx = TransactionBuilder::new();
    let coins = make_move_vec::<Receiving, _>(&mut tx, None, [coin(6), coin(7)]).unwrap();
    royalty_pool::pool::recover_coins(
        &mut tx,
        &d,
        vec![usdc(), usdc()],
        ObjectRef::shared(addr(1), 2),
        coins,
    )
    .unwrap();
    let p = ptb(tx);
    assert_eq!(
        p.inputs,
        vec![
            Input::Receiving(sui_sdk_types::ObjectReference::new(
                addr(6),
                6,
                Digest::ZERO
            )),
            Input::Receiving(sui_sdk_types::ObjectReference::new(
                addr(7),
                7,
                Digest::ZERO
            )),
            shared(addr(1), 2, true),
        ]
    );
    match &p.commands[0] {
        Command::MakeMoveVector(v) => {
            assert_eq!(v.elements, vec![Argument::Input(0), Argument::Input(1)]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        call(&p.commands[1]).arguments,
        vec![Argument::Input(2), Argument::Result(0)]
    );
}
