use crate::{
    debug::{
        Debugger, ReferenceKind, Type, TypeDeclaration,
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

/// Provides custom expansion for a [`Variable`].
///
/// The first registered provider whose [`matches`](Self::matches) returns
/// `true` for a variable wins; its [`children`](Self::children) result replaces
/// the default structure/pointer expansion. Providers that only need to alter
/// matching can rely on the default `children` implementation, which yields the
/// raw structural view.
pub trait VariableProvider {
    fn matches(&self, value: &Variable) -> bool;

    fn children(&self, value: &Variable, dbg: &Debugger) -> Vec<Variable> {
        value.children(dbg.info())
    }

    fn display(&self, value: &Variable, dbg: &Debugger) -> String {
        value.display(dbg.info())
    }
}

/// A typed value backed by one or more DWARF location pieces.
///
/// `pieces` describes where the bytes live (memory address, embedded value,
/// register, …); `ty` describes how to interpret them.
#[derive(Clone)]
pub struct Variable {
    name: String,
    pieces: Vec<gimli::Piece<R>>,
    ty: Type,
}

impl Variable {
    pub fn new(name: String, pieces: Vec<gimli::Piece<R>>, ty: Type) -> Self {
        Self { name, pieces, ty }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn ty(&self) -> &Type {
        &self.ty
    }

    pub fn address(&self) -> Option<GlobalAddress> {
        let piece = self.pieces.first()?;
        match &piece.location {
            gimli::Location::Address { address } => Some(GlobalAddress(*address)),
            _ => None,
        }
    }

    pub fn addr_value(&self, info: &DebugInfo) -> Option<u64> {
        Some(read_ptr(info, self.address()?.0))
    }

    /// Human-readable type name (e.g. `int`, `Point`, `int*`).
    pub fn type_name(&self) -> String {
        self.ty.name()
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
            Some(TypeDeclaration::Referential { target, kind, .. }) => match kind {
                ReferenceKind::Pointer => match self.address() {
                    Some(addr) => addr.to_string(),
                    None => "<unavailable>".into(),
                },
                ReferenceKind::Reference | ReferenceKind::Temporary => {
                    let Some(addr) = self.address() else {
                        return "<unavailable>".into();
                    };
                    Variable::new(
                        self.name.clone(),
                        vec![addr_piece(read_ptr(info, addr.0))],
                        self.ty.child(*target),
                    )
                    .display(info)
                }
            },
            _ => "<unavailable>".into(),
        }
    }

    /// Expands a compound value into named child variables.
    ///
    /// Returns an empty vector for scalars / unsupported aggregates.
    pub fn children(&self, info: &DebugInfo) -> Vec<Variable> {
        match self.ty.resolved() {
            Some(TypeDeclaration::Structure { members, .. }) => {
                let Some(base) = self.address() else {
                    return Vec::new();
                };
                let mut out = Vec::with_capacity(members.len());
                for member in members {
                    let Some(name) = member.name.clone() else {
                        continue;
                    };
                    let offset = match &member.location {
                        Some(super::Value::Constant(o)) => *o,
                        None => 0,
                        Some(super::Value::Expr(_)) => continue,
                    };
                    let addr = (base.0 as i64).wrapping_add(offset) as u64;
                    out.push(Variable::new(
                        name,
                        vec![addr_piece(addr)],
                        self.ty.child(member.ty),
                    ));
                }
                out
            }
            Some(TypeDeclaration::Referential { target, kind, .. }) => {
                let is_ptr = matches!(kind, ReferenceKind::Pointer);
                let Some(addr) = self.address() else {
                    return Vec::new();
                };
                let target_addr = read_ptr(info, addr.0);
                if is_ptr && target_addr == 0 {
                    return Vec::new();
                }
                let target_type = self.ty.child(*target);
                let piece = addr_piece(target_addr);
                if is_ptr && matches!(target_type.resolved(), Some(TypeDeclaration::Scalar { .. }))
                {
                    return vec![Variable::new(
                        format!("*{}", self.name),
                        vec![piece],
                        target_type,
                    )];
                }
                Variable::new(self.name.clone(), vec![piece], target_type).children(info)
            }
            _ => Vec::new(),
        }
    }
}

// Reusable for various formatter that follow the same access pattern
pub trait VariableSliceExt {
    fn find(&self, name: &str) -> Option<&Variable>;
}

impl VariableSliceExt for [Variable] {
    fn find(&self, name: &str) -> Option<&Variable> {
        self.iter().find(|v| v.name() == name)
    }
}

/// Reads `len` bytes addressed by the first piece (memory or immediate).
///
/// Returns `None` if the location is empty or unsupported.
fn read_value_bytes(info: &DebugInfo, pieces: &[gimli::Piece<R>], len: usize) -> Option<Vec<u8>> {
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

pub(super) fn read_ptr(info: &DebugInfo, addr: u64) -> u64 {
    u32::from_le_bytes(read_main_memory(info, addr, 4).try_into().unwrap_or([0; 4])) as u64
}

pub(super) fn addr_piece(address: u64) -> gimli::Piece<R> {
    gimli::Piece {
        size_in_bits: None,
        bit_offset: None,
        location: gimli::Location::Address { address },
    }
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
