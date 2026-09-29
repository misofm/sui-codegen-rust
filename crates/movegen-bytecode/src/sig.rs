//! Signature tokens kept in their bytecode encoding (B-SIG-1).
//!
//! A token tree is serialized in pre-order: each node is a tag byte plus ULEB operands, followed
//! by its children. [`read_token`] validates a tree once while reading the module and records
//! its byte slice; afterwards trees are walked without allocating ([`nodes`], [`Tokens`]) or
//! decoded into an owned type only when an ABI is produced.

use crate::cursor::Cursor;
use crate::error::Error;
use crate::model::Token;

const TYPE_PARAMETER_INDEX_MAX: u64 = 65536;
const TYPE_PARAMETER_COUNT_MAX: u64 = 255;
/// Mysten's explicit-stack loader rejects once 256 constructors are pending (a stack of 257
/// entries), so at most 255 type constructors may be open at once.
const OPEN_CONSTRUCTORS_MAX: usize = 255;

/// One node of a token tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Node {
    Bool,
    U8,
    U16,
    U32,
    U64,
    U128,
    U256,
    Address,
    Signer,
    Vector,
    Reference,
    MutableReference,
    /// A datatype handle with no type arguments.
    Datatype(u16),
    /// A datatype handle applied to `n` type arguments (the next `n` trees).
    Instantiation(u16, u8),
    TypeParameter(u16),
}

impl Node {
    fn children(self) -> usize {
        match self {
            Node::Vector | Node::Reference | Node::MutableReference => 1,
            Node::Instantiation(_, n) => usize::from(n),
            _ => 0,
        }
    }
}

/// Reads one node header, validating it.
fn read_node(c: &mut Cursor<'_>) -> Result<Node, Error> {
    Ok(match c.u8("signature token")? {
        0x1 => Node::Bool,
        0x2 => Node::U8,
        0x3 => Node::U64,
        0x4 => Node::U128,
        0x5 => Node::Address,
        0x6 => Node::Reference,
        0x7 => Node::MutableReference,
        0x8 => Node::Datatype(c.index("datatype handle")?),
        0x9 => {
            let idx = c.uleb_max("type parameter index", TYPE_PARAMETER_INDEX_MAX)?;
            Node::TypeParameter(
                u16::try_from(idx)
                    .map_err(|_| Error::BadSignature("type parameter index overflow"))?,
            )
        }
        0xA => Node::Vector,
        0xB => {
            let handle = c.index("datatype handle")?;
            let arity = c.uleb_max("type parameter count", TYPE_PARAMETER_COUNT_MAX)?;
            if arity == 0 {
                return Err(Error::BadSignature("datatype instantiation with arity 0"));
            }
            Node::Instantiation(handle, u8::try_from(arity).expect("bounded"))
        }
        0xC => Node::Signer,
        0xD => Node::U16,
        0xE => Node::U32,
        0xF => Node::U256,
        _ => return Err(Error::BadSignature("unknown token")),
    })
}

/// Reads and validates one token tree; returns its encoding (B-SIG-1).
pub(crate) fn read_token<'a>(c: &mut Cursor<'a>) -> Result<Token<'a>, Error> {
    let start = c.position();
    // Children still expected by each open constructor (≤ 255 each, ≤ 255 open).
    let mut open = [0u8; OPEN_CONSTRUCTORS_MAX];
    let mut depth = 0usize;
    loop {
        let children = read_node(c)?.children();
        if children > 0 {
            if depth == OPEN_CONSTRUCTORS_MAX {
                return Err(Error::BadSignature("maximum nesting depth reached"));
            }
            open[depth] = u8::try_from(children).expect("≤ 255");
            depth += 1;
            continue;
        }
        // A complete subtree: close every constructor it completes.
        loop {
            if depth == 0 {
                return Ok(Token(c.slice_from(start)));
            }
            open[depth - 1] -= 1;
            if open[depth - 1] == 0 {
                depth -= 1;
            } else {
                break;
            }
        }
    }
}

/// Decodes a node header from validated bytes. Returns `None` only on bytes that were never
/// validated (a bug), so callers treat it as unreachable-by-construction but never panic.
fn decode_node(bytes: &[u8], pos: &mut usize) -> Option<Node> {
    fn uleb(bytes: &[u8], pos: &mut usize) -> Option<u64> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = *bytes.get(*pos)?;
            *pos += 1;
            value |= u64::from(byte & 0x7f).checked_shl(shift)?;
            if byte & 0x80 == 0 {
                return Some(value);
            }
            shift += 7;
        }
    }
    let tag = *bytes.get(*pos)?;
    *pos += 1;
    Some(match tag {
        0x1 => Node::Bool,
        0x2 => Node::U8,
        0x3 => Node::U64,
        0x4 => Node::U128,
        0x5 => Node::Address,
        0x6 => Node::Reference,
        0x7 => Node::MutableReference,
        0x8 => Node::Datatype(u16::try_from(uleb(bytes, pos)?).ok()?),
        0x9 => Node::TypeParameter(u16::try_from(uleb(bytes, pos)?).ok()?),
        0xA => Node::Vector,
        0xB => {
            let handle = u16::try_from(uleb(bytes, pos)?).ok()?;
            Node::Instantiation(handle, u8::try_from(uleb(bytes, pos)?).ok()?)
        }
        0xC => Node::Signer,
        0xD => Node::U16,
        0xE => Node::U32,
        0xF => Node::U256,
        _ => return None,
    })
}

/// Every node of a token tree, in pre-order, without allocating.
pub(crate) fn nodes(token: Token<'_>) -> impl Iterator<Item = Node> + '_ {
    let mut pos = 0;
    std::iter::from_fn(move || {
        if pos < token.0.len() {
            decode_node(token.0, &mut pos)
        } else {
            None
        }
    })
}

/// The largest type-parameter index in a token (for B-IDX-1 type-parameter checks).
pub(crate) fn max_type_parameter(token: Token<'_>) -> Option<u16> {
    nodes(token)
        .filter_map(|n| {
            if let Node::TypeParameter(i) = n {
                Some(i)
            } else {
                None
            }
        })
        .max()
}

/// The tokens of a signature, in order.
#[derive(Clone, Debug)]
pub struct Tokens<'a> {
    bytes: &'a [u8],
    remaining: usize,
}

impl<'a> Tokens<'a> {
    pub(crate) fn new(bytes: &'a [u8], len: usize) -> Self {
        Self {
            bytes,
            remaining: len,
        }
    }
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        if self.remaining == 0 {
            return None;
        }
        // Skip one tree: `pending` counts nodes still to read.
        let mut pos = 0;
        let mut pending = 1usize;
        while pending > 0 {
            let node = decode_node(self.bytes, &mut pos)?;
            pending = pending - 1 + node.children();
        }
        let (token, rest) = self.bytes.split_at(pos);
        self.bytes = rest;
        self.remaining -= 1;
        Some(Token(token))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for Tokens<'_> {}

/// Reads the nodes of a validated token tree in pre-order; decoders recurse over it.
pub(crate) struct NodeReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> NodeReader<'a> {
    pub(crate) fn new(token: Token<'a>) -> Self {
        Self {
            bytes: token.0,
            pos: 0,
        }
    }

    pub(crate) fn next(&mut self) -> Result<Node, Error> {
        decode_node(self.bytes, &mut self.pos).ok_or(Error::BadSignature("corrupt token"))
    }
}
