use std::rc::Rc;

use crate::{
    debug::{
        Debugger, ReferenceKind, Type, TypeDeclaration,
        dwarf::{Die, R, Visit},
    },
    types::{DebugInfo, GlobalAddress},
};

#[allow(unused_imports)]
use crate::debug::formatters::VariableFormatter;

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
pub struct Variable {
    pub(crate) dbg: Rc<Debugger>,
    pub(crate) name: String,
    pub(crate) pieces: Vec<gimli::Piece<R>>,
    pub(crate) ty: Type,
}

impl Variable {
    /// Duplicates the variable with a new name, contents, and type.
    fn copy(&self, name: String, pieces: Vec<gimli::Piece<R>>, ty: Type) -> Self {
        Self {
            dbg: self.dbg.clone(),
            name,
            pieces,
            ty,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn ty(&self) -> &Type {
        &self.ty
    }

    /// Returns the address of this variable.
    /// If this variable has no known address, returns [None].
    pub fn address(&self) -> Option<GlobalAddress> {
        let piece = self.pieces.first()?;
        match &piece.location {
            gimli::Location::Address { address } => Some(GlobalAddress(*address)),
            _ => None,
        }
    }

    /// Reads `len` bytes from the start of this variable's value.
    /// If exactly `len` bytes cannot be read, returns [None].
    pub fn read(&self, len: usize) -> Option<Vec<u8>> {
        // TODO: Handling for multi-piece value, not just `first()`
        let piece = self.pieces.first()?;
        let mut bytes = match &piece.location {
            gimli::Location::Address { address } => {
                read_main_memory(self.dbg.info(), *address, len)
            }
            gimli::Location::Value { value } => value_to_le_bytes(*value, len),
            gimli::Location::Bytes { value } => value.to_slice().ok()?.to_vec(),
            _ => Vec::default(),
        };

        if bytes.len() < len {
            return None;
        }

        bytes.resize(len, 0);
        Some(bytes)
    }

    /// Returns the actual address stored by a pointer.
    ///
    /// For example, in the following snippet:
    ///
    /// ```cpp
    /// int* x = (int*) 0xBA5EBA11;
    /// ```
    ///
    /// [Variable::pointer_value] would return `Some(GlobalAddress(0xBA5EBA11))` for
    /// the [Variable] corresponding to `x`.
    pub fn pointer_value(&self) -> Option<GlobalAddress> {
        if let Some(bytes) = self.read(4) {
            Some(u32::from_le_bytes(bytes.try_into().ok()?).into())
        } else {
            None
        }
    }

    /// Renders this value to a string using the default logic.
    /// Use [Self::formatted_display] to use any matching [VariableFormatter] instead.
    pub fn display(&self) -> String {
        match self.ty.resolved() {
            Some(TypeDeclaration::Scalar {
                byte_size,
                encoding,
                ..
            }) => {
                let Some(bytes) = self.read(*byte_size as usize) else {
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
                    let Some(addr) = self.pointer_value() else {
                        return "<unavailable>".into();
                    };
                    self.copy(self.name.clone(), addr.pieces(), self.ty.child(*target))
                        .display()
                }
            },
            _ => "<unavailable>".into(),
        }
    }

    /// Expands this variable into its raw children using the default logic.
    /// Use [Self::formatted_children] to use any matching [VariableFormatter] instead.
    pub fn children(&self) -> Vec<Variable> {
        match self.ty.resolved() {
            Some(TypeDeclaration::Structure { members, .. }) => {
                // TODO: What is structure not located in memory? E.g. stored in pieces instead
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
                    out.push(self.copy(name, vec![addr_piece(addr)], self.ty.child(member.ty)));
                }
                out
            }
            Some(TypeDeclaration::Referential { target, kind, .. }) => {
                let is_ptr = matches!(kind, ReferenceKind::Pointer);
                let Some(addr) = self.pointer_value() else {
                    return Vec::new();
                };
                if is_ptr && addr.is_null() {
                    return Vec::new();
                }
                let target_type = self.ty.child(*target);
                if is_ptr && matches!(target_type.resolved(), Some(TypeDeclaration::Scalar { .. }))
                {
                    return vec![self.copy(format!("*{}", self.name), addr.pieces(), target_type)];
                }
                self.copy(self.name.clone(), addr.pieces(), target_type)
                    .children()
            }
            _ => Vec::new(),
        }
    }

    /// Returns the children of this variable as if it were an array.
    ///
    /// `start` is the index of the first child to retrieve, and `count` is how many to fetch.
    /// Fewer than `count` elements may be returned if the debugger is unable to fetch that many
    /// due to OOB accesses or known array bounds.
    ///
    /// For pointer types, this will treat a `T*` as if it were a `T[]`.
    pub fn indexed_children(&self, start: usize, count: usize) -> Vec<Variable> {
        match self.ty.resolved() {
            Some(TypeDeclaration::Referential { target, kind, .. })
                if matches!(kind, ReferenceKind::Pointer) =>
            {
                let Some(base) = self.pointer_value() else {
                    return Vec::new();
                };

                if base.is_null() {
                    return Vec::new();
                }

                let elem_ty = self.ty.child(*target);
                let Some(elem_size) = elem_ty.byte_size() else {
                    return Vec::new();
                };

                let elem_size = elem_size as usize;
                if elem_size == 0 {
                    return Vec::new();
                }

                let mut result = Vec::new();

                for i in start..start + count {
                    // Compute start address of element
                    let offset = base.0 as usize + i * elem_size;

                    // Ensure that entire element is in-bounds
                    if offset + elem_size >= self.dbg.info().memory.byte_size() {
                        break;
                    }

                    result.push(self.copy(
                        format!("[{i}]"),
                        GlobalAddress(offset as u64).pieces(),
                        elem_ty.clone(),
                    ));
                }

                result
            }
            _ => {
                // For all other types, let's simply query the children and return a slice.
                // Note that this will be inefficient for large arrays, but simpler to implement
                let children = self.formatted_children();
                children.into_iter().skip(start).take(count).collect()
            }
        }
    }

    /// Renders this variable to a string using any matching [VariableFormatter].
    ///
    /// Be careful calling this inside of a [VariableFormatter::display] implementation
    /// that you do not cause an infinite loop.
    pub fn formatted_display(&self) -> String {
        for formatter in &self.dbg.formatters {
            if formatter.matches(self)
                && let Some(result) = formatter.display(self)
            {
                return result;
            }
        }

        self.display()
    }

    /// Expands this variable into its children any matching [VariableFormatter].
    ///
    /// Be careful calling this inside of a [VariableFormatter::children] implementation
    /// that you do not cause an infinite loop.
    pub fn formatted_children(&self) -> Vec<Variable> {
        for formatter in &self.dbg.formatters {
            if formatter.matches(self)
                && let Some(result) = formatter.children(self)
            {
                return result;
            }
        }
        self.children()
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

impl GlobalAddress {
    /// Returns the pieces for a variable located at this address.
    pub fn pieces(&self) -> Vec<gimli::Piece<R>> {
        vec![gimli::Piece {
            size_in_bits: None,
            bit_offset: None,
            location: gimli::Location::Address {
                address: (*self).into(),
            },
        }]
    }
}
