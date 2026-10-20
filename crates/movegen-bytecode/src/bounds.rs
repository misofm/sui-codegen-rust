//! B-IDX-1: a port of `move-binary-format`'s `BoundsChecker` (`check_bounds.rs` at Sui
//! `testnet-v1.82.1`). Table-level checks run once every non-code table is read; code is checked
//! while it streams past ([`Code`]), so no instruction is ever stored. Signature checks scan the
//! bytecode encoding in place.

use crate::error::Error;
use crate::model::{CompiledModule, Token};
use crate::sig::{Node, max_type_parameter, nodes};

fn bounds(what: &'static str, index: usize, len: usize) -> Result<(), Error> {
    if index < len {
        Ok(())
    } else {
        Err(Error::OutOfBounds { what, index, len })
    }
}

fn arity_mismatch(expected: usize, got: usize) -> Error {
    Error::IllFormedType(format!(
        "datatype expects {expected} type arguments, got {got}"
    ))
}

/// Per-signature largest type-parameter index (Mysten's `signature_max_type_params`).
pub(crate) struct Memo(Vec<Option<u16>>);

impl Memo {
    /// Every type parameter in signature `sig` is below `count`.
    fn check(&self, sig: u16, count: usize) -> Result<(), Error> {
        match self.0.get(usize::from(sig)).copied().flatten() {
            Some(max) if usize::from(max) >= count => Err(Error::OutOfBounds {
                what: "type parameter",
                index: usize::from(max),
                len: count,
            }),
            _ => Ok(()),
        }
    }
}

/// `check_type`: datatype indices in bounds, arity matches; returns the largest type parameter.
fn check_type(m: &CompiledModule<'_>, token: Token<'_>) -> Result<Option<u16>, Error> {
    let mut max = None;
    for node in nodes(token) {
        match node {
            Node::TypeParameter(i) => max = max.max(Some(i)),
            Node::Datatype(h) => {
                bounds("datatype handle", usize::from(h), m.datatype_handles.len())?;
                let arity = m.datatype_handles[usize::from(h)].arity();
                if arity != 0 {
                    return Err(arity_mismatch(arity, 0));
                }
            }
            Node::Instantiation(h, n) => {
                bounds("datatype handle", usize::from(h), m.datatype_handles.len())?;
                let arity = m.datatype_handles[usize::from(h)].arity();
                if arity != usize::from(n) {
                    return Err(arity_mismatch(arity, usize::from(n)));
                }
            }
            _ => {}
        }
    }
    Ok(max)
}

/// `check_type_parameter`: every type parameter index is below `count`.
fn check_type_parameters(token: Token<'_>, count: usize) -> Result<(), Error> {
    match max_type_parameter(token) {
        Some(max) => bounds("type parameter", usize::from(max), count),
        None => Ok(()),
    }
}

/// Every check in `BoundsChecker::verify_impl` except function definitions.
pub(crate) fn check_tables(m: &CompiledModule<'_>) -> Result<Memo, Error> {
    if m.module_handles.is_empty() {
        return Err(Error::OutOfBounds {
            what: "module handle (none present)",
            index: 0,
            len: 0,
        });
    }
    let mut memo = Vec::with_capacity(m.signatures.len());
    for signature in &m.signatures {
        let mut max = None;
        for token in signature.tokens() {
            max = max.max(check_type(m, token)?);
        }
        memo.push(max);
    }
    let memo = Memo(memo);
    for c in &m.constants {
        check_type(m, c.type_)?;
    }
    let idents = m.identifiers.len();
    for h in &m.module_handles {
        bounds("address", usize::from(h.address), m.address_count())?;
        bounds("identifier", usize::from(h.name), idents)?;
    }
    bounds(
        "self module handle",
        usize::from(m.self_handle),
        m.module_handles.len(),
    )?;
    for h in &m.datatype_handles {
        bounds(
            "module handle",
            usize::from(h.module),
            m.module_handles.len(),
        )?;
        bounds("identifier", usize::from(h.name), idents)?;
    }
    for h in &m.function_handles {
        bounds(
            "module handle",
            usize::from(h.module),
            m.module_handles.len(),
        )?;
        bounds("identifier", usize::from(h.name), idents)?;
        bounds("signature", usize::from(h.parameters), m.signatures.len())?;
        bounds("signature", usize::from(h.return_), m.signatures.len())?;
        memo.check(h.parameters, h.arity())?;
        memo.check(h.return_, h.arity())?;
    }
    for &(owner, field) in &m.field_handles {
        bounds("struct definition", usize::from(owner), m.struct_defs.len())?;
        let count = m
            .struct_fields(&m.struct_defs[usize::from(owner)])
            .map_or(0, <[_]>::len);
        bounds("field", usize::from(field), count)?;
    }
    for h in &m.friends {
        bounds("address", usize::from(h.address), m.address_count())?;
        bounds("identifier", usize::from(h.name), idents)?;
    }
    for &(def, sig) in &m.struct_instantiations {
        bounds("struct definition", usize::from(def), m.struct_defs.len())?;
        bounds("signature", usize::from(sig), m.signatures.len())?;
    }
    for &(handle, sig) in &m.function_instantiations {
        bounds(
            "function handle",
            usize::from(handle),
            m.function_handles.len(),
        )?;
        bounds("signature", usize::from(sig), m.signatures.len())?;
    }
    for &(handle, sig) in &m.field_instantiations {
        bounds("field handle", usize::from(handle), m.field_handles.len())?;
        bounds("signature", usize::from(sig), m.signatures.len())?;
    }
    for def in &m.struct_defs {
        bounds(
            "datatype handle",
            usize::from(def.handle),
            m.datatype_handles.len(),
        )?;
        let count = m.datatype_handles[usize::from(def.handle)].arity();
        for f in m.struct_fields(def).into_iter().flatten() {
            bounds("identifier", usize::from(f.name), idents)?;
            check_type(m, f.type_)?;
            check_type_parameters(f.type_, count)?;
        }
    }
    for def in &m.enum_defs {
        bounds(
            "datatype handle",
            usize::from(def.handle),
            m.datatype_handles.len(),
        )?;
        let count = m.datatype_handles[usize::from(def.handle)].arity();
        for v in m.enum_variants(def) {
            bounds("identifier", usize::from(v.name), idents)?;
            for f in m.variant_fields(v) {
                bounds("identifier", usize::from(f.name), idents)?;
                check_type(m, f.type_)?;
                check_type_parameters(f.type_, count)?;
            }
        }
    }
    for &(def, sig) in &m.enum_instantiations {
        bounds("enum definition", usize::from(def), m.enum_defs.len())?;
        bounds("signature", usize::from(sig), m.signatures.len())?;
    }
    for &(def, tag) in &m.variant_handles {
        bounds("enum definition", usize::from(def), m.enum_defs.len())?;
        bounds(
            "variant tag",
            usize::from(tag),
            m.enum_defs[usize::from(def)].variant_count(),
        )?;
    }
    for &(inst, tag) in &m.variant_instantiation_handles {
        bounds(
            "enum instantiation",
            usize::from(inst),
            m.enum_instantiations.len(),
        )?;
        let (def, _) = m.enum_instantiations[usize::from(inst)];
        bounds(
            "variant tag",
            usize::from(tag),
            m.enum_defs[usize::from(def)].variant_count(),
        )?;
    }
    Ok(memo)
}

/// Checks function definitions and their code (`check_function_def`, `check_code`).
pub(crate) struct CodeChecker<'k, 'a> {
    m: &'k CompiledModule<'a>,
    memo: &'k Memo,
}

impl<'k, 'a> CodeChecker<'k, 'a> {
    pub(crate) fn new(m: &'k CompiledModule<'a>, memo: &'k Memo) -> Self {
        Self { m, memo }
    }

    pub(crate) fn function_handle(&self, handle: u16) -> Result<(), Error> {
        bounds(
            "function handle",
            usize::from(handle),
            self.m.function_handles.len(),
        )
    }

    /// Starts checking the code unit of function `handle` (already bounds-checked).
    pub(crate) fn begin(
        &self,
        handle: u16,
        locals: u16,
        code_len: u64,
    ) -> Result<Code<'k, 'a>, Error> {
        let m = self.m;
        let handle = &m.function_handles[usize::from(handle)];
        bounds(
            "signature",
            usize::from(handle.parameters),
            m.signatures.len(),
        )?;
        let parameters = m.signatures[usize::from(handle.parameters)].len();
        bounds("signature", usize::from(locals), m.signatures.len())?;
        let locals_count = m.signatures[usize::from(locals)]
            .len()
            .saturating_add(parameters);
        if locals_count > usize::from(u8::MAX) {
            return Err(Error::TooLarge {
                what: "locals",
                value: locals_count as u64,
                max: 255,
            });
        }
        let type_params = handle.arity();
        self.memo.check(locals, type_params)?;
        Ok(Code {
            m,
            memo: self.memo,
            type_params,
            locals: locals_count as u64,
            len: code_len,
            max_jump_table_index: None,
        })
    }
}

/// Streaming checks for one code unit.
pub(crate) struct Code<'k, 'a> {
    m: &'k CompiledModule<'a>,
    memo: &'k Memo,
    type_params: usize,
    locals: u64,
    len: u64,
    max_jump_table_index: Option<u64>,
}

fn bounds_u64(what: &'static str, index: u64, len: u64) -> Result<(), Error> {
    if index < len {
        Ok(())
    } else {
        Err(Error::OutOfBounds {
            what,
            index: usize::try_from(index).unwrap_or(usize::MAX),
            len: usize::try_from(len).unwrap_or(usize::MAX),
        })
    }
}

impl Code<'_, '_> {
    pub(crate) fn branch(&mut self, target: u64) -> Result<(), Error> {
        bounds_u64("branch target", target, self.len)
    }

    pub(crate) fn constant(&mut self, i: u16) -> Result<(), Error> {
        bounds("constant", usize::from(i), self.m.constants.len())
    }

    pub(crate) fn local(&mut self, i: u64) -> Result<(), Error> {
        bounds_u64("local", i, self.locals)
    }

    pub(crate) fn field_handle(&mut self, i: u16) -> Result<(), Error> {
        bounds("field handle", usize::from(i), self.m.field_handles.len())
    }

    pub(crate) fn field_instantiation(&mut self, i: u16) -> Result<(), Error> {
        bounds(
            "field instantiation",
            usize::from(i),
            self.m.field_instantiations.len(),
        )?;
        self.memo.check(
            self.m.field_instantiations[usize::from(i)].1,
            self.type_params,
        )
    }

    pub(crate) fn function_handle(&mut self, i: u16) -> Result<(), Error> {
        bounds(
            "function handle",
            usize::from(i),
            self.m.function_handles.len(),
        )
    }

    pub(crate) fn function_instantiation(&mut self, i: u16) -> Result<(), Error> {
        bounds(
            "function instantiation",
            usize::from(i),
            self.m.function_instantiations.len(),
        )?;
        self.memo.check(
            self.m.function_instantiations[usize::from(i)].1,
            self.type_params,
        )
    }

    pub(crate) fn struct_definition(&mut self, i: u16) -> Result<(), Error> {
        bounds(
            "struct definition",
            usize::from(i),
            self.m.struct_defs.len(),
        )
    }

    pub(crate) fn struct_instantiation(&mut self, i: u16) -> Result<(), Error> {
        bounds(
            "struct instantiation",
            usize::from(i),
            self.m.struct_instantiations.len(),
        )?;
        self.memo.check(
            self.m.struct_instantiations[usize::from(i)].1,
            self.type_params,
        )
    }

    pub(crate) fn vector_signature(&mut self, i: u16) -> Result<(), Error> {
        bounds("signature", usize::from(i), self.m.signatures.len())?;
        self.memo.check(i, self.type_params)
    }

    pub(crate) fn variant_handle(&mut self, i: u64) -> Result<(), Error> {
        bounds_u64("variant handle", i, self.m.variant_handles.len() as u64)
    }

    pub(crate) fn variant_instantiation(&mut self, i: u64) -> Result<(), Error> {
        let len = self.m.variant_instantiation_handles.len() as u64;
        bounds_u64("variant instantiation", i, len)?;
        let (inst, _) = self.m.variant_instantiation_handles[usize::try_from(i).expect("< len")];
        self.memo.check(
            self.m.enum_instantiations[usize::from(inst)].1,
            self.type_params,
        )
    }

    /// Jump-table indices refer to tables that follow the code; checked in [`Code::finish`].
    pub(crate) fn jump_table_index(&mut self, i: u64) {
        self.max_jump_table_index = self.max_jump_table_index.max(Some(i));
    }

    /// A jump table over enum `head` must have one branch per variant.
    pub(crate) fn jump_table(&mut self, head: u16, branches: u64) -> Result<(), Error> {
        bounds("enum definition", usize::from(head), self.m.enum_defs.len())?;
        let variants = self.m.enum_defs[usize::from(head)].variant_count() as u64;
        if branches != variants {
            return Err(Error::IllFormedType(format!(
                "jump table has {branches} branches for {variants} variants"
            )));
        }
        Ok(())
    }

    pub(crate) fn finish(self, jump_tables: u64) -> Result<(), Error> {
        match self.max_jump_table_index {
            Some(max) => bounds_u64("jump table", max, jump_tables),
            None => Ok(()),
        }
    }
}
