use crate::{
    debug::{
        Type, TypeDeclaration,
        dwarf::{Die, R, Visit},
    },
    types::{DebugInfo, GlobalAddress},
};
use gimli::Reader;
use gimli::read::Expression;
use wasm_bindgen::JsCast;

/// Gets all live variables at the given PC.
///
/// `die` should be a reference to a subprogram,
/// and `pc` should be an offset within the wasm code segment.
pub fn get_variables<'a>(die: &Die<'a>, pc: GlobalAddress) -> Vec<Die<'a>> {
    assert!(
        die.tag() == gimli::DW_TAG_subprogram,
        "get_variables requires subprogram die"
    );

    let mut result: Vec<Die<'a>> = vec![];

    die.traverse(|child| {
        let tag = child.tag();

        match tag {
            // `DW_TAG_variable` is what modern DWARF (and clang) emit for
            // locals; `DW_TAG_local_variable` is a legacy alias kept for
            // older producers.
            gimli::DW_TAG_formal_parameter
            | gimli::DW_TAG_variable
            | gimli::DW_TAG_local_variable => {
                result.push(child);
            }

            _ => {
                if let Some((low, high)) = child.addr_range() {
                    if pc < low || pc >= high {
                        return Visit::SkipChildren;
                    }
                }
            }
        }

        Visit::Continue
    });

    result
}

/// Gets the location expression for a variable at the given PC
pub fn get_location(die: &Die<'_>, pc: GlobalAddress) -> Option<Expression<R>> {
    die.expression(gimli::DW_AT_location, pc)
}

/// A typed value backed by one or more DWARF location pieces.
///
/// `pieces` describes where the bytes live (memory address, embedded value,
/// register, …); `ty` describes how to interpret them.
#[derive(Clone)]
pub struct Value {
    pieces: Vec<gimli::Piece<R>>,
    ty: Type,
}

impl Value {
    pub fn new(pieces: Vec<gimli::Piece<R>>, ty: Type) -> Self {
        Self { pieces, ty }
    }

    pub fn ty(&self) -> &Type {
        &self.ty
    }

    /// Human-readable type name (e.g. `int`, `Point`, `int*`).
    pub fn type_name(&self) -> String {
        self.ty.name()
    }

    /// Returns `true` if this value should be expandable into child variables
    /// (currently: structs/unions/classes, after peeling typedefs/cv-quals).
    pub fn has_children(&self) -> bool {
        matches!(self.ty.resolved(), Some(TypeDeclaration::Structure { .. }))
    }

    /// Renders this value for the DAP `value` field.
    ///
    /// - Scalars are decoded according to their DWARF encoding.
    /// - Compound types render as e.g. `Point { ... }`; their fields are
    ///   reachable via [`Self::children`].
    pub fn display(&self, info: &DebugInfo) -> String {
        match self.ty.resolved() {
            Some(TypeDeclaration::Scalar {
                byte_size,
                encoding,
                ..
            }) => {
                let Some(bytes) = read_value_bytes(info, &self.pieces, *byte_size as usize) else {
                    return "<unavailable>".into();
                };
                format_scalar(&bytes, *encoding, *byte_size)
            }
            Some(TypeDeclaration::Structure { name, .. }) => {
                let label = name.as_deref().unwrap_or("");
                format!("{label} {{ ... }}")
            }
            Some(TypeDeclaration::Referential { .. }) => match read_address(&self.pieces) {
                Some(addr) => format!("0x{:x}", addr),
                None => "<unavailable>".into(),
            },
            _ => "<unavailable>".into(),
        }
    }

    /// Expands a compound value into named child variables.
    ///
    /// Returns an empty vector for scalars / unsupported aggregates.
    pub fn children(&self, _info: &DebugInfo) -> Vec<super::Variable> {
        let Some(TypeDeclaration::Structure { members, .. }) = self.ty.resolved() else {
            return Vec::new();
        };

        let Some(base) = read_address(&self.pieces) else {
            return Vec::new();
        };

        let mut out = Vec::with_capacity(members.len());
        for member in members {
            let Some(name) = member.name.clone() else {
                continue;
            };
            let offset = match &member.location {
                Some(super::MemberLocation::Offset(o)) => *o,
                // Member at the start when no location is specified (common for unions)
                None => 0,
                // Expression-based offsets aren't supported yet
                Some(super::MemberLocation::Expr(_)) => continue,
            };
            let addr = (base as i64).wrapping_add(offset) as u64;
            let piece = gimli::Piece {
                size_in_bits: None,
                bit_offset: None,
                location: gimli::Location::Address { address: addr },
            };
            let value = Value::new(vec![piece], self.ty.child(member.ty));
            out.push(super::Variable { name, value });
        }
        out
    }
}

/// Reads `len` bytes addressed by the first piece (memory or immediate).
///
/// Returns `None` if the location is empty or unsupported.
fn read_value_bytes(
    info: &DebugInfo,
    pieces: &[gimli::Piece<R>],
    len: usize,
) -> Option<Vec<u8>> {
    let piece = pieces.first()?;
    match &piece.location {
        gimli::Location::Address { address } => Some(read_main_memory(info, *address, len)),
        gimli::Location::Value { value } => Some(value_to_le_bytes(*value, len)),
        gimli::Location::Bytes { value } => {
            let mut buf = value.to_slice().ok()?.to_vec();
            buf.resize(len, 0);
            Some(buf)
        }
        _ => None,
    }
}

/// Returns the address pointed to by the first piece, if any.
fn read_address(pieces: &[gimli::Piece<R>]) -> Option<u64> {
    let piece = pieces.first()?;
    match &piece.location {
        gimli::Location::Address { address } => Some(*address),
        _ => None,
    }
}

fn read_main_memory(info: &DebugInfo, address: u64, len: usize) -> Vec<u8> {
    let buffer = info.memory.memory.buffer();
    let buffer = buffer.unchecked_ref::<js_sys::ArrayBuffer>();
    let total = buffer.byte_length() as usize;
    if address as usize >= total {
        return vec![0; len];
    }
    let avail = (total - address as usize).min(len);
    let view = js_sys::Uint8Array::new_with_byte_offset_and_length(
        &buffer.into(),
        address as u32,
        avail as u32,
    );
    let mut out = vec![0u8; len];
    view.copy_to(&mut out[..avail]);
    out
}

fn value_to_le_bytes(value: gimli::Value, len: usize) -> Vec<u8> {
    let raw: u64 = match value {
        gimli::Value::Generic(v) => v,
        gimli::Value::I8(v) => v as i64 as u64,
        gimli::Value::U8(v) => v as u64,
        gimli::Value::I16(v) => v as i64 as u64,
        gimli::Value::U16(v) => v as u64,
        gimli::Value::I32(v) => v as i64 as u64,
        gimli::Value::U32(v) => v as u64,
        gimli::Value::I64(v) => v as u64,
        gimli::Value::U64(v) => v,
        gimli::Value::F32(_) | gimli::Value::F64(_) => 0,
    };
    let bytes = raw.to_le_bytes();
    let mut out = vec![0u8; len];
    let copy = len.min(8);
    out[..copy].copy_from_slice(&bytes[..copy]);
    out
}

fn format_scalar(bytes: &[u8], encoding: gimli::DwAte, byte_size: u64) -> String {
    let size = byte_size as usize;
    if bytes.len() < size {
        return "<truncated>".into();
    }
    match encoding {
        gimli::DW_ATE_signed | gimli::DW_ATE_signed_char => match size {
            1 => (bytes[0] as i8).to_string(),
            2 => i16::from_le_bytes(bytes[..2].try_into().unwrap()).to_string(),
            4 => i32::from_le_bytes(bytes[..4].try_into().unwrap()).to_string(),
            8 => i64::from_le_bytes(bytes[..8].try_into().unwrap()).to_string(),
            _ => "<unsupported size>".into(),
        },
        gimli::DW_ATE_unsigned | gimli::DW_ATE_unsigned_char => match size {
            1 => bytes[0].to_string(),
            2 => u16::from_le_bytes(bytes[..2].try_into().unwrap()).to_string(),
            4 => u32::from_le_bytes(bytes[..4].try_into().unwrap()).to_string(),
            8 => u64::from_le_bytes(bytes[..8].try_into().unwrap()).to_string(),
            _ => "<unsupported size>".into(),
        },
        gimli::DW_ATE_boolean => {
            let v = bytes.iter().take(size).any(|&b| b != 0);
            (if v { "true" } else { "false" }).into()
        }
        gimli::DW_ATE_float => match size {
            4 => f32::from_le_bytes(bytes[..4].try_into().unwrap()).to_string(),
            8 => f64::from_le_bytes(bytes[..8].try_into().unwrap()).to_string(),
            _ => "<unsupported float size>".into(),
        },
        gimli::DW_ATE_UTF | gimli::DW_ATE_ASCII => match size {
            1 => format!("{:?}", bytes[0] as char),
            _ => "<unsupported char size>".into(),
        },
        _ => "<unsupported encoding>".into(),
    }
}
