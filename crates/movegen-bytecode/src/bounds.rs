//! B-IDX-1: a port of `move-binary-format`'s `BoundsChecker` (`check_bounds.rs` at Sui
//! `mainnet-v1.80.1`). A module this accepts has every index in bounds — tables, signatures and
//! code operands — so anything read from it is well-formed.

use crate::error::Error;
use crate::model::{CodeUnit, CompiledModule, FunctionDefinition, Operand, SignatureToken};

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

/// Runs every check in `BoundsChecker::verify_impl`'s order.
pub(crate) fn check(m: &CompiledModule) -> Result<(), Error> {
    if m.module_handles.is_empty() {
        return Err(Error::OutOfBounds {
            what: "module handle (none present)",
            index: 0,
            len: 0,
        });
    }
    // Signatures, memoizing each signature's largest type parameter.
    let mut max_type_param = Vec::with_capacity(m.signatures.len());
    for signature in &m.signatures {
        let mut max: Option<u16> = None;
        for token in signature {
            max = max.max(check_type(m, token)?);
        }
        max_type_param.push(max);
    }
    let sig_type_params = |sig: u16, count: usize| -> Result<(), Error> {
        match max_type_param.get(usize::from(sig)).copied().flatten() {
            Some(max) if usize::from(max) >= count => Err(Error::OutOfBounds {
                what: "type parameter",
                index: usize::from(max),
                len: count,
            }),
            _ => Ok(()),
        }
    };
    for c in &m.constants {
        check_type(m, &c.type_)?;
    }
    for h in &m.module_handles {
        bounds("address", usize::from(h.address), m.addresses.len())?;
        bounds("identifier", usize::from(h.name), m.identifiers.len())?;
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
        bounds("identifier", usize::from(h.name), m.identifiers.len())?;
    }
    for h in &m.function_handles {
        bounds(
            "module handle",
            usize::from(h.module),
            m.module_handles.len(),
        )?;
        bounds("identifier", usize::from(h.name), m.identifiers.len())?;
        bounds("signature", usize::from(h.parameters), m.signatures.len())?;
        bounds("signature", usize::from(h.return_), m.signatures.len())?;
        sig_type_params(h.parameters, h.type_parameters.len())?;
        sig_type_params(h.return_, h.type_parameters.len())?;
    }
    for &(owner, field) in &m.field_handles {
        bounds("struct definition", usize::from(owner), m.struct_defs.len())?;
        let count = m.struct_defs[usize::from(owner)]
            .fields
            .as_ref()
            .map_or(0, Vec::len);
        bounds("field", usize::from(field), count)?;
    }
    for h in &m.friends {
        bounds("address", usize::from(h.address), m.addresses.len())?;
        bounds("identifier", usize::from(h.name), m.identifiers.len())?;
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
        let count = m.datatype_handles[usize::from(def.handle)]
            .type_parameters
            .len();
        for f in def.fields.iter().flatten() {
            bounds("identifier", usize::from(f.name), m.identifiers.len())?;
            check_type(m, &f.type_)?;
            check_type_parameters(&f.type_, count)?;
        }
    }
    for def in &m.enum_defs {
        bounds(
            "datatype handle",
            usize::from(def.handle),
            m.datatype_handles.len(),
        )?;
        let count = m.datatype_handles[usize::from(def.handle)]
            .type_parameters
            .len();
        for v in &def.variants {
            bounds("identifier", usize::from(v.name), m.identifiers.len())?;
            for f in &v.fields {
                bounds("identifier", usize::from(f.name), m.identifiers.len())?;
                check_type(m, &f.type_)?;
                check_type_parameters(&f.type_, count)?;
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
            m.enum_defs[usize::from(def)].variants.len(),
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
            m.enum_defs[usize::from(def)].variants.len(),
        )?;
    }
    for def in &m.function_defs {
        check_function(m, def, &sig_type_params)?;
    }
    Ok(())
}

/// `check_type`: datatype indices in bounds, arity matches; returns the largest type parameter.
fn check_type(m: &CompiledModule, token: &SignatureToken) -> Result<Option<u16>, Error> {
    Ok(match token {
        SignatureToken::TypeParameter(i) => Some(*i),
        SignatureToken::Vector(t)
        | SignatureToken::Reference(t)
        | SignatureToken::MutableReference(t) => check_type(m, t)?,
        SignatureToken::Datatype(h) => {
            bounds("datatype handle", usize::from(*h), m.datatype_handles.len())?;
            let arity = m.datatype_handles[usize::from(*h)].type_parameters.len();
            if arity != 0 {
                return Err(arity_mismatch(arity, 0));
            }
            None
        }
        SignatureToken::DatatypeInstantiation(h, args) => {
            bounds("datatype handle", usize::from(*h), m.datatype_handles.len())?;
            let arity = m.datatype_handles[usize::from(*h)].type_parameters.len();
            if arity != args.len() {
                return Err(arity_mismatch(arity, args.len()));
            }
            let mut max = None;
            for a in args {
                max = max.max(check_type(m, a)?);
            }
            max
        }
        _ => None,
    })
}

/// `check_type_parameter`: every type parameter index is below `count`.
fn check_type_parameters(token: &SignatureToken, count: usize) -> Result<(), Error> {
    match token {
        SignatureToken::TypeParameter(i) => bounds("type parameter", usize::from(*i), count),
        SignatureToken::Vector(t)
        | SignatureToken::Reference(t)
        | SignatureToken::MutableReference(t) => check_type_parameters(t, count),
        SignatureToken::DatatypeInstantiation(_, args) => args
            .iter()
            .try_for_each(|a| check_type_parameters(a, count)),
        _ => Ok(()),
    }
}

fn check_function(
    m: &CompiledModule,
    def: &FunctionDefinition,
    sig_type_params: &impl Fn(u16, usize) -> Result<(), Error>,
) -> Result<(), Error> {
    bounds(
        "function handle",
        usize::from(def.handle),
        m.function_handles.len(),
    )?;
    let Some(code) = &def.code else { return Ok(()) };
    let handle = &m.function_handles[usize::from(def.handle)];
    bounds(
        "signature",
        usize::from(handle.parameters),
        m.signatures.len(),
    )?;
    let parameters = m.signatures[usize::from(handle.parameters)].len();
    check_code(
        m,
        code,
        handle.type_parameters.len(),
        parameters,
        sig_type_params,
    )
}

fn check_code(
    m: &CompiledModule,
    unit: &CodeUnit,
    type_params: usize,
    parameters: usize,
    sig_type_params: &impl Fn(u16, usize) -> Result<(), Error>,
) -> Result<(), Error> {
    bounds("signature", usize::from(unit.locals), m.signatures.len())?;
    let locals = m.signatures[usize::from(unit.locals)]
        .len()
        .saturating_add(parameters);
    if locals > usize::from(u8::MAX) {
        return Err(Error::TooLarge {
            what: "locals",
            value: locals as u64,
            max: 255,
        });
    }
    sig_type_params(unit.locals, type_params)?;
    let code_len = unit.code.len();
    for op in &unit.code {
        match *op {
            Operand::None => {}
            Operand::Branch(target) => bounds("branch target", usize::from(target), code_len)?,
            Operand::Constant(i) => bounds("constant", usize::from(i), m.constants.len())?,
            Operand::Local(i) => bounds("local", usize::from(i), locals)?,
            Operand::FieldHandle(i) => {
                bounds("field handle", usize::from(i), m.field_handles.len())?;
            }
            Operand::FieldInstantiation(i) => {
                bounds(
                    "field instantiation",
                    usize::from(i),
                    m.field_instantiations.len(),
                )?;
                sig_type_params(m.field_instantiations[usize::from(i)].1, type_params)?;
            }
            Operand::FunctionHandle(i) => {
                bounds("function handle", usize::from(i), m.function_handles.len())?;
            }
            Operand::FunctionInstantiation(i) => {
                bounds(
                    "function instantiation",
                    usize::from(i),
                    m.function_instantiations.len(),
                )?;
                sig_type_params(m.function_instantiations[usize::from(i)].1, type_params)?;
            }
            Operand::StructDefinition(i) => {
                bounds("struct definition", usize::from(i), m.struct_defs.len())?;
            }
            Operand::StructInstantiation(i) => {
                bounds(
                    "struct instantiation",
                    usize::from(i),
                    m.struct_instantiations.len(),
                )?;
                sig_type_params(m.struct_instantiations[usize::from(i)].1, type_params)?;
            }
            Operand::VectorSignature(i) => {
                bounds("signature", usize::from(i), m.signatures.len())?;
                sig_type_params(i, type_params)?;
            }
            Operand::VariantHandle(i) => {
                bounds("variant handle", usize::from(i), m.variant_handles.len())?;
            }
            Operand::VariantInstantiation(i) => {
                bounds(
                    "variant instantiation",
                    usize::from(i),
                    m.variant_instantiation_handles.len(),
                )?;
                let (inst, _) = m.variant_instantiation_handles[usize::from(i)];
                sig_type_params(m.enum_instantiations[usize::from(inst)].1, type_params)?;
            }
            Operand::JumpTable(i) => bounds("jump table", usize::from(i), unit.jump_tables.len())?,
        }
    }
    for (head, offsets) in &unit.jump_tables {
        bounds("enum definition", usize::from(*head), m.enum_defs.len())?;
        let variants = m.enum_defs[usize::from(*head)].variants.len();
        if offsets.len() != variants {
            return Err(Error::IllFormedType(format!(
                "jump table has {} branches for {variants} variants",
                offsets.len()
            )));
        }
        for offset in offsets {
            bounds("jump table target", usize::from(*offset), code_len)?;
        }
    }
    Ok(())
}
