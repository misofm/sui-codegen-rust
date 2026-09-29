//! Typed PTB building on top of `sui-transaction-builder` (C rules in docs/SPEC.md).
//!
//! Generated call functions take `impl IntoInput<T>` per parameter and materialize inputs *inside
//! the call, in parameter order* — matching `@mysten/sui`, where `tx.moveCall` resolves argument
//! thunks in order. Pure values are never de-duplicated (`pure_bytes_unique`, like TS). Shared
//! objects take their mutability from the parameter's reference kind and are OR-merged by the
//! builder on reuse (like TS's `isUsedAsMutable`), unless the caller fixed it explicitly.

use std::marker::PhantomData;

use serde::Serialize;
use sui_sdk_types::{Address, Digest, Identifier, MoveCall, TypeTag};
use sui_transaction_builder::{Argument, Function, ObjectInput, TransactionBuilder};

use crate::{Deployment, Error};

/// A PTB value of Move type `T` (a generated binding type, or [`Any`] for type parameters).
pub struct Arg<T: ?Sized> {
    arg: Argument,
    _type: PhantomData<fn() -> T>,
}

impl<T: ?Sized> Clone for Arg<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: ?Sized> Copy for Arg<T> {}

impl<T: ?Sized> std::fmt::Debug for Arg<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Arg").field(&self.arg).finish()
    }
}

impl<T: ?Sized> Arg<T> {
    /// Wraps a builder argument. The caller asserts it holds a `T`.
    pub fn new(arg: Argument) -> Self {
        Self {
            arg,
            _type: PhantomData,
        }
    }

    /// The builder argument.
    pub fn argument(self) -> Argument {
        self.arg
    }

    /// Reinterprets the value's type (an escape hatch; the chain checks types at execution).
    pub fn cast<U: ?Sized>(self) -> Arg<U> {
        Arg::new(self.arg)
    }
}

impl<T: ?Sized> From<Arg<T>> for Argument {
    fn from(arg: Arg<T>) -> Argument {
        arg.arg
    }
}

/// Marker for values whose Move type is a type parameter of the call. Objects ([`ObjectRef`],
/// or an `Arg<T>` of an object type), pure values, and `Arg<Any>` convert into it; cast other
/// results with [`Arg::cast`].
#[derive(Debug)]
pub enum Any {}

/// Marker for a `Receiving<T>` parameter: only a [`ReceivingRef`] converts into it, so passing a
/// normal object where a receiving reference is expected (or vice versa) does not compile.
#[derive(Debug)]
pub struct Receive<T: ?Sized>(PhantomData<fn() -> T>, std::convert::Infallible);

/// An object to pass by reference or value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectRef {
    /// A shared object. `mutable: None` lets each use decide (OR-merged); `Some(false)` refuses
    /// mutable or by-value use.
    Shared {
        /// Object id.
        id: Address,
        /// Initial shared version.
        initial_shared_version: u64,
        /// Explicit mutability, if fixed by the caller.
        mutable: Option<bool>,
    },
    /// An address-owned or immutable object.
    Owned {
        /// Object id.
        id: Address,
        /// Version.
        version: u64,
        /// Digest.
        digest: Digest,
    },
}

/// An object sent to another object's address, for a `Receiving<T>` parameter (or an element of
/// a `vector<Receiving<T>>` built with [`make_move_vec`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceivingRef {
    /// Object id.
    pub id: Address,
    /// Version.
    pub version: u64,
    /// Digest.
    pub digest: Digest,
}

impl ObjectRef {
    /// A shared object whose mutability is decided by how it is used.
    pub fn shared(id: Address, initial_shared_version: u64) -> Self {
        ObjectRef::Shared {
            id,
            initial_shared_version,
            mutable: None,
        }
    }
}

/// How a parameter's value is supplied.
#[derive(Debug)]
pub enum Input<T: ?Sized> {
    /// An existing PTB value (an input already added, or a previous result).
    Arg(Arg<T>),
    /// BCS bytes of a pure value, added as a new pure input.
    Pure(Vec<u8>),
    /// An object, added (or merged) as an object input.
    Object(ObjectRef),
    /// An object to receive, added as a receiving input.
    Receiving(ReceivingRef),
}

/// Values that can be supplied for a parameter of Move type `T`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be passed for a Move parameter of type `{T}`",
    label = "not a `{T}`",
    note = "pass an `Arg<{T}>` (a previous result), an `ObjectRef` if `{T}` is an object, a Rust value if it is pure, or a `ReceivingRef` for a `Receiving<_>` parameter"
)]
pub trait IntoInput<T: ?Sized> {
    /// The input.
    fn into_input(self) -> Input<T>;
}

impl<T: ?Sized> IntoInput<T> for Arg<T> {
    fn into_input(self) -> Input<T> {
        Input::Arg(self)
    }
}

/// Types that are valid pure arguments (C-PURE-1): primitives, `address`, `ID`, strings, and
/// `Option`/`vector` of pure types.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a pure Move value",
    note = "pure values are primitives, `Address`, `Id`, strings, and `Option`/`Vec` of pure values"
)]
pub trait Pure: Serialize {}

impl<T: Pure> IntoInput<T> for T {
    fn into_input(self) -> Input<T> {
        Input::Pure(bcs::to_bytes(&self).expect("pure values serialize"))
    }
}

impl<T: Pure> IntoInput<T> for &T {
    fn into_input(self) -> Input<T> {
        Input::Pure(bcs::to_bytes(self).expect("pure values serialize"))
    }
}

macro_rules! pure {
    ($($t:ty),*) => { $(impl Pure for $t {})* };
}
pure!(
    bool,
    u8,
    u16,
    u32,
    u64,
    u128,
    sui_sdk_types::U256,
    Address,
    String
);
impl<T: Pure> Pure for Option<T> {}
impl<T: Pure> Pure for Vec<T> {}

/// Types with the `key` ability: they can be passed as objects.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a Move object type (it lacks `key`), so an `ObjectRef` cannot be passed for it",
    label = "an `ObjectRef` was passed here",
    note = "`Receiving<_>` parameters take a `ReceivingRef`; other values must be previous results (`Arg<_>`)"
)]
pub trait Key {}

impl<T: Key + ?Sized> IntoInput<T> for ObjectRef {
    fn into_input(self) -> Input<T> {
        Input::Object(self)
    }
}

impl<T: ?Sized> IntoInput<Receive<T>> for ReceivingRef {
    fn into_input(self) -> Input<Receive<T>> {
        Input::Receiving(self)
    }
}

impl IntoInput<crate::Receiving> for ReceivingRef {
    fn into_input(self) -> Input<crate::Receiving> {
        Input::Receiving(self)
    }
}

// Type parameters (C-GEN-1): the instantiation is a runtime type argument, so any object or pure
// value is accepted. (Coherent because `Any` is neither `Key` nor `Pure`.)
impl IntoInput<Any> for ObjectRef {
    fn into_input(self) -> Input<Any> {
        Input::Object(self)
    }
}

impl<T: Key + ?Sized> IntoInput<Any> for Arg<T> {
    fn into_input(self) -> Input<Any> {
        Input::Arg(self.cast())
    }
}

impl<T: Pure> IntoInput<Any> for T {
    fn into_input(self) -> Input<Any> {
        Input::Pure(bcs::to_bytes(&self).expect("pure values serialize"))
    }
}

/// Why a call could not be added.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CallError {
    /// A pure value or object was supplied where only a PTB result is possible.
    #[error("parameter {0} needs a PTB value (a previous result), not a new input")]
    NeedsValue(usize),
    /// An object was supplied for a pure parameter.
    #[error("parameter {0} is a pure value, not an object")]
    NotPure(usize),
    /// A pure value was supplied for an object parameter.
    #[error("parameter {0} is an object")]
    NotObject(usize),
    /// An explicitly immutable shared object was used mutably or by value.
    #[error("parameter {0} needs a mutable object but the shared object was marked immutable")]
    Immutable(usize),
    /// A receiving reference was supplied for a normal object parameter, or vice versa.
    #[error("parameter {0}: receiving references are only valid for `Receiving<T>` parameters")]
    Receiving(usize),
    /// Wrong number of type arguments.
    #[error("{function} takes {expected} type arguments, got {got}")]
    TypeArguments {
        /// Function.
        function: &'static str,
        /// Expected count.
        expected: usize,
        /// Supplied count.
        got: usize,
    },
    /// The deployment lacks something the call needs.
    #[error(transparent)]
    Deployment(#[from] Error),
}

/// How a parameter is passed (C-KIND-1). Used by generated calls and by inspectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    /// A pure value.
    Pure,
    /// An object by `&`.
    ObjectImm,
    /// An object by `&mut`.
    ObjectMut,
    /// An object by value.
    ObjectByValue,
    /// A `Receiving<T>`.
    Receiving,
    /// A system object injected by the call (`clock`, `random`, …) with this mutability.
    System(&'static str, bool),
    /// A non-pure, non-object value: must be a previous result.
    Value,
    /// A type parameter: pure or object depending on the instantiation.
    Generic,
}

/// Materializes parameter `index` of kind `kind` (C-ORD-1: called in parameter order).
///
/// A thin generic shim: the type is erased immediately so the body is compiled once, not once
/// per Move type (keeps generated bindings small, notably in wasm).
#[inline]
pub fn input<T: ?Sized>(
    tx: &mut TransactionBuilder,
    index: usize,
    kind: ArgKind,
    value: Input<T>,
) -> Result<Argument, CallError> {
    let raw = match value {
        Input::Arg(arg) => RawInput::Arg(arg.argument()),
        Input::Pure(bytes) => RawInput::Pure(bytes),
        Input::Object(obj) => RawInput::Object(obj),
        Input::Receiving(obj) => RawInput::Receiving(obj),
    };
    input_raw(tx, index, kind, raw)
}

enum RawInput {
    Arg(Argument),
    Pure(Vec<u8>),
    Object(ObjectRef),
    Receiving(ReceivingRef),
}

fn input_raw(
    tx: &mut TransactionBuilder,
    index: usize,
    kind: ArgKind,
    value: RawInput,
) -> Result<Argument, CallError> {
    Ok(match (kind, value) {
        (_, RawInput::Arg(arg)) => arg,
        (ArgKind::Pure | ArgKind::Generic, RawInput::Pure(bytes)) => tx.pure_bytes_unique(bytes),
        (
            ArgKind::ObjectImm | ArgKind::ObjectMut | ArgKind::ObjectByValue | ArgKind::Generic,
            RawInput::Object(obj),
        ) => {
            let mutable = !matches!(kind, ArgKind::ObjectImm);
            match obj {
                ObjectRef::Shared {
                    id,
                    initial_shared_version,
                    mutable: explicit,
                } => {
                    if explicit == Some(false) && mutable {
                        return Err(CallError::Immutable(index));
                    }
                    let effective = explicit.unwrap_or(mutable) || mutable;
                    tx.object(ObjectInput::shared(id, initial_shared_version, effective))
                }
                ObjectRef::Owned {
                    id,
                    version,
                    digest,
                } => tx.object(ObjectInput::owned(id, version, digest)),
            }
        }
        (
            ArgKind::Receiving,
            RawInput::Receiving(ReceivingRef {
                id,
                version,
                digest,
            }),
        ) => tx.object(ObjectInput::receiving(id, version, digest)),
        (ArgKind::Receiving, RawInput::Object(_)) | (_, RawInput::Receiving(_)) => {
            return Err(CallError::Receiving(index));
        }
        (ArgKind::Pure, RawInput::Object(_)) => return Err(CallError::NotPure(index)),
        (
            ArgKind::ObjectImm | ArgKind::ObjectMut | ArgKind::ObjectByValue | ArgKind::Receiving,
            RawInput::Pure(_),
        ) => {
            return Err(CallError::NotObject(index));
        }
        (ArgKind::Value | ArgKind::System(..), _) => return Err(CallError::NeedsValue(index)),
    })
}

/// Builds a `vector<T>` from elements (C-VEC-1): previous results, pure values, objects (by
/// value), or receiving references, materialized in order. `element_type` is required by the
/// chain when the vector is empty or its elements are pure.
pub fn make_move_vec<T, I: IntoInput<T>>(
    tx: &mut TransactionBuilder,
    element_type: Option<TypeTag>,
    elements: impl IntoIterator<Item = I>,
) -> Result<Arg<Vec<T>>, CallError> {
    let elements = elements
        .into_iter()
        .enumerate()
        .map(|(i, element)| match element.into_input() {
            Input::Receiving(r) => input_raw(tx, i, ArgKind::Receiving, RawInput::Receiving(r)),
            other => input(tx, i, ArgKind::Generic, other),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Arg::new(tx.make_move_vec(element_type, elements)))
}

/// Injects a system object (C-SYS-1): `clock` (`0x6`), `random` (`0x8`), `deny_list` (`0x403`),
/// `accumulator_root` (`0xacc`), `sui_system_state` (`0x5`), `coin_registry` (`0xc`),
/// `display_registry` (`0xd`), `address_alias_state` (`0xa`).
pub fn system_object(
    tx: &mut TransactionBuilder,
    d: &Deployment,
    name: &'static str,
    mutable: bool,
) -> Result<Argument, CallError> {
    let object = d.system_object(name)?;
    Ok(tx.object(ObjectInput::shared(
        object.id,
        object.initial_shared_version,
        mutable,
    )))
}

/// A Move function call target — the data inspectors compare against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveTarget {
    /// Package (the deployment's `published_at`).
    pub package: Address,
    /// Module.
    pub module: &'static str,
    /// Function.
    pub function: &'static str,
    /// Type arguments.
    pub type_arguments: Vec<TypeTag>,
}

impl MoveTarget {
    /// Whether `call` targets exactly this function with these type arguments.
    pub fn matches(&self, call: &MoveCall) -> bool {
        call.package == self.package
            && call.module.as_str() == self.module
            && call.function.as_str() == self.function
            && call.type_arguments == self.type_arguments
    }

    /// Appends the call and returns its result.
    pub fn call(self, tx: &mut TransactionBuilder, args: Vec<Argument>) -> Argument {
        let function = Function::new(
            self.package,
            Identifier::from_static(self.module),
            Identifier::from_static(self.function),
        )
        .with_type_args(self.type_arguments);
        tx.move_call(function, args)
    }
}

/// Checks a call's type-argument count.
pub fn type_arguments<const N: usize>(
    function: &'static str,
    type_arguments: Vec<TypeTag>,
) -> Result<Vec<TypeTag>, CallError> {
    if type_arguments.len() == N {
        Ok(type_arguments)
    } else {
        Err(CallError::TypeArguments {
            function,
            expected: N,
            got: type_arguments.len(),
        })
    }
}

/// Splits a multi-value result into its `NestedResult`s.
pub fn nested(result: Argument, count: usize) -> Vec<Argument> {
    result.to_nested(count)
}
