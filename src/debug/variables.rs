use std::cell::OnceCell;
use std::ops::Range;

use crate::{
    debug::{
        Debugger, ReferenceKind, Type, TypeDeclaration,
        dwarf::{Die, R, Visit},
        formatters::{ChildCounts, VariableFormatter},
    },
    types::GlobalAddress,
    util::WeakRef,
};

use anyhow::Result;

use gimli::Reader;
use gimli::read::Expression;

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
    debugger: WeakRef<Debugger>,
    name: String,
    pieces: Vec<gimli::Piece<R>>,
    ty: Type,
    cache: VariableCache,
}

#[derive(Clone, Default)]
struct VariableCache {
    named: OnceCell<Vec<Variable>>,
    indexed: OnceCell<Vec<Variable>>,
}

impl VariableCache {
    pub fn named(&self, var: &Variable) -> &Vec<Variable> {
        self.named
            .get_or_init(|| compute_default_named_children(var))
    }

    pub fn indexed(&self, var: &Variable) -> &Vec<Variable> {
        self.indexed
            .get_or_init(|| compute_default_indexed_children(var))
    }
}

impl Variable {
    pub fn new(
        debugger: WeakRef<Debugger>,
        name: String,
        pieces: Vec<gimli::Piece<R>>,
        ty: Type,
    ) -> Self {
        Variable {
            debugger: debugger.clone(),
            name,
            pieces,
            ty,
            cache: Default::default(),
        }
    }

    pub(crate) fn debugger(&self) -> Option<&Debugger> {
        self.debugger.as_deref()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn ty(&self) -> &Type {
        &self.ty
    }

    /// Changes the name of this variable.
    pub fn with_name(self, name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..self
        }
    }

    /// Changes the type of this variable.
    pub fn with_type(self, ty: &Type) -> Self {
        Self {
            ty: ty.clone(),
            cache: Default::default(),
            ..self
        }
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
        // TODO: This function is very important, albeit poorly tested/understood.
        // It will fail under optimizing compilers, which may store values as collections of pieces
        // rather than the simple formats that this method assumes.
        //
        // More cases should be added for multi-piece values.
        // These should be rigorously tested under a test harness.

        // TODO: Handling for multi-piece value, not just `first()`
        let piece = self.pieces.first()?;
        let mut bytes = match &piece.location {
            gimli::Location::Address { address } => self
                .debugger()?
                .memory()
                .read_memory(GlobalAddress(*address), len),
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

    /// Get the unsigned value of this variable.
    /// If the value cannot be represented as an unsigned value, returns [None].
    ///
    /// * For signed integral types, returns [None] for negative values.
    /// * For character types, returns the unsigned value of the code point.
    /// * For floating point types, returns [None].
    /// * For pointer types, returns the pointer's value.
    pub fn unsigned_value(&self) -> Option<u64> {
        match self.ty.resolved()? {
            TypeDeclaration::Scalar {
                byte_size,
                encoding,
                ..
            } => {
                let size = *byte_size as usize;
                let bytes = self.read(size)?;
                match *encoding {
                    gimli::DW_ATE_signed | gimli::DW_ATE_signed_char => {
                        let signed = match size {
                            1 => i64::from(bytes[0] as i8),
                            2 => i64::from(i16::from_le_bytes(bytes[..2].try_into().ok()?)),
                            4 => i64::from(i32::from_le_bytes(bytes[..4].try_into().ok()?)),
                            8 => i64::from(i64::from_le_bytes(bytes[..8].try_into().ok()?)),
                            _ => return None,
                        };
                        u64::try_from(signed).ok()
                    }
                    gimli::DW_ATE_unsigned | gimli::DW_ATE_unsigned_char => match size {
                        1 => Some(bytes[0] as u64),
                        2 => Some(u16::from_le_bytes(bytes[..2].try_into().ok()?) as u64),
                        4 => Some(u32::from_le_bytes(bytes[..4].try_into().ok()?) as u64),
                        8 => Some(u64::from_le_bytes(bytes[..8].try_into().ok()?)),
                        _ => None,
                    },
                    gimli::DW_ATE_UTF | gimli::DW_ATE_ASCII if size == 1 => Some(bytes[0] as u64),
                    gimli::DW_ATE_float | gimli::DW_ATE_boolean => None,
                    _ => None,
                }
            }
            TypeDeclaration::Referential {
                kind: ReferenceKind::Pointer,
                ..
            } => self.pointer_value().map(u64::from),
            _ => None,
        }
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

    pub fn display(&self) -> Result<String> {
        Ok(match self.ty().resolved() {
            Some(TypeDeclaration::Scalar {
                byte_size,
                encoding,
                ..
            }) => {
                let Some(bytes) = self.read(*byte_size as usize) else {
                    return Ok("<unavailable>".into());
                };
                format_scalar(&bytes, *encoding, *byte_size)
            }
            Some(TypeDeclaration::Structure { .. }) => {
                if let Some(addr) = self.address() {
                    format!("@{addr}")
                } else {
                    String::default()
                }
            }
            Some(TypeDeclaration::Referential { target, kind, .. }) => match kind {
                ReferenceKind::Pointer => match self.address() {
                    Some(addr) => addr.to_string(),
                    None => "<unavailable>".into(),
                },
                ReferenceKind::Reference | ReferenceKind::Temporary => {
                    return self
                        .child_at_offset(0)
                        .with_type(&self.ty.child(*target))
                        .display();
                }
            },
            _ => "<unavailable>".into(),
        })
    }

    pub fn num_children(&self) -> Result<ChildCounts> {
        Ok(ChildCounts::mixed(
            self.cache.indexed(self).len(),
            self.cache.named(self).len(),
        ))
    }

    pub fn indexed_children(&self, range: Range<usize>) -> Result<Vec<Variable>> {
        match self.ty.resolved() {
            Some(TypeDeclaration::Referential { target, kind, .. })
                if matches!(kind, ReferenceKind::Pointer) =>
            {
                let Some(base) = self.pointer_value() else {
                    return Ok(Vec::new());
                };
                if base.is_null() {
                    return Ok(Vec::new());
                }

                let elem_ty = self.ty.child(*target);
                let Some(elem_size) = elem_ty.byte_size() else {
                    return Ok(Vec::new());
                };

                let elem_size = elem_size as usize;
                if elem_size == 0 {
                    return Ok(Vec::new());
                }

                let mut result = Vec::new();
                for i in range {
                    result.push(
                        self.child_at_offset(i * elem_size)
                            .with_name(&format!("[{i}]"))
                            .with_type(&elem_ty),
                    );
                }

                Ok(result)
            }
            _ => Ok(self.cache.indexed(self)[range].to_vec()),
        }
    }

    pub fn named_children(&self, range: Range<usize>) -> Result<Vec<Variable>> {
        Ok(self.cache.named(self)[range].to_vec())
    }

    /// Gets a child named `name` in this variable.
    ///
    /// For example, for structural types, `name` might be the name of a member within this variable.
    ///
    /// Returns [None] if no such child is found.
    pub fn child_with_name(&self, name: &str) -> Option<Variable> {
        self.cache
            .named(self)
            .into_iter()
            .find(|child| child.name() == name)
            .cloned()
    }

    /// Returns a child of this variable with byte offset `offset`.
    ///
    /// The returned variable will have an empty name and the same type as this one.
    /// You may change the resulting name and type using [Variable::with_name] and [Variable::with_type].
    ///
    /// Roughly speaking, the offsets correspond to the same as "space" as the variable's children.
    /// E.g. an `int* ptr`'s `child_at_offset(0)` would return `ptr[0]`, a structured variable
    /// `my_struct_t x`'s `child_at_offset(0)` would return the first member, and so on.
    pub fn child_at_offset(&self, offset: usize) -> Variable {
        // TODO: This function is very important, albeit poorly tested/understood.
        // It will fail under optimizing compilers, which may store values as collections of pieces
        // rather than the simple formats that this method assumes.
        //
        // More cases should be added for multi-piece values.
        // These should be rigorously tested under a test harness.

        // Dereference pointer types
        let mut pieces = match self.ty.resolved() {
            Some(TypeDeclaration::Referential { .. }) => self
                .pointer_value()
                .filter(|a| !a.is_null())
                .map(|a| a.pieces())
                .unwrap_or_default(),
            _ => self.pieces.clone(),
        };

        // Skip `ofs` bytes through the piece vector
        let mut ofs = offset as u64;
        while let Some(p) = pieces.first() {
            let Some(bits) = p.size_in_bits else { break };
            let piece_bytes = ((bits) + 7) / 8;
            if ofs < piece_bytes {
                break;
            }
            ofs -= piece_bytes;
            pieces.remove(0);
        }

        // Add `ofs` bytes to the first piece
        if let Some(p) = pieces.first_mut() {
            match &mut p.location {
                gimli::Location::Address { address } => *address += ofs,
                _ => p.bit_offset = Some(p.bit_offset.unwrap_or(0) + ofs * 8),
            }
        }

        // Clone the variable with the new contents
        Self {
            debugger: self.debugger.clone(),
            name: Default::default(),
            pieces,
            ty: self.ty.clone(),
            cache: Default::default(),
        }
    }

    fn formatter(&self) -> Option<&dyn VariableFormatter> {
        self.debugger()?
            .formatters
            .iter()
            .find(|formatter| formatter.matches(self))
            .map(|formatter| formatter.as_ref())
    }

    pub fn formatted_display(&self) -> Result<String> {
        match self.formatter() {
            Some(formatter) => formatter.display(self),
            None => self.display(),
        }
    }

    pub fn formatted_num_children(&self) -> Result<ChildCounts> {
        match self.formatter() {
            Some(formatter) => formatter.num_children(self),
            None => self.num_children(),
        }
    }

    pub fn formatted_indexed_children(&self, range: Range<usize>) -> Result<Vec<Variable>> {
        match self.formatter() {
            Some(formatter) => formatter.indexed_children(self, range),
            None => self.indexed_children(range),
        }
    }

    pub fn formatted_named_children(&self, range: Range<usize>) -> Result<Vec<Variable>> {
        match self.formatter() {
            Some(formatter) => formatter.named_children(self, range),
            None => self.named_children(range),
        }
    }
}

/// Computes the default named children of a variable.
fn compute_default_named_children(var: &Variable) -> Vec<Variable> {
    match var.ty.resolved() {
        Some(TypeDeclaration::Structure { members, .. }) => {
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
                out.push(
                    var.child_at_offset(offset as usize)
                        .with_name(&name)
                        .with_type(&var.ty.child(member.ty)),
                );
            }
            out
        }
        Some(TypeDeclaration::Referential { target, kind, .. }) => {
            let is_ptr = matches!(kind, ReferenceKind::Pointer);
            let Some(addr) = var.pointer_value() else {
                return Vec::new();
            };
            if is_ptr && addr.is_null() {
                return Vec::new();
            }
            let target_type = var.ty.child(*target);
            if is_ptr && matches!(target_type.resolved(), Some(TypeDeclaration::Scalar { .. })) {
                return vec![
                    var.child_at_offset(0)
                        .with_name(&format!("*{}", var.name))
                        .with_type(&target_type),
                ];
            }

            let inner = var
                .child_at_offset(0)
                .with_name(var.name())
                .with_type(&target_type);
            compute_default_named_children(&inner)
        }
        _ => Vec::new(),
    }
}

/// Computes the default indexed children of a variable.
fn compute_default_indexed_children(_var: &Variable) -> Vec<Variable> {
    Vec::new()
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
