use crate::{
    debug::dwarf::{Die, DieReference, Visit},
    types::GlobalAddress,
};
use gimli::read::Expression;
use std::collections::HashMap;

use super::R;

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
            gimli::DW_TAG_formal_parameter | gimli::DW_TAG_local_variable => {
                result.push(child);
            }

            _ => {
                if let Some(low) = child.low_pc()
                    && pc < low
                {
                    return Visit::SkipChildren;
                }

                if let Some(high) = child.high_pc()
                    && pc >= high
                {
                    return Visit::SkipChildren;
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

type TypeId = gimli::DieReference;

pub struct Value {
    inner: Vec<gimli::Piece<R>>,
    ty: TypeId,
}

pub struct TypeGraph {
    types: HashMap<TypeId, TypeDeclaration>,
}

#[derive(Clone, Debug)]
pub enum MemberLocation {
    Offset(i64),
    Expr(gimli::Expression<R>),
}

#[derive(Clone, Debug)]
pub struct StructureMember {
    pub location: Option<MemberLocation>,
    pub name: Option<String>,
    pub ty: TypeId,
}

#[derive(Clone, Debug)]
pub enum ReferenceKind {
    Pointer,
    Reference,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Modifier {
    TypeDef,
    Const,
    Volatile,
    Atomic,
    Restrict,
}

#[derive(Clone, Debug)]
pub enum ArrayBound {
    Expr(gimli::Expression<R>),
    Count(i64),
}

#[derive(Clone, Debug)]
pub enum TypeDeclaration {
    Scalar {
        name: String,
        byte_size: u64,
        encoding: gimli::DwAte,
    },
    Array {
        byte_size: Option<u64>,
        element_type: TypeId,
        lower_bound: ArrayBound,
        upper_bound: Option<ArrayBound>,
    },
    Referential {
        target: TypeId,
        kind: ReferenceKind,
    },
    Structure {
        name: Option<String>,
        byte_size: u64,
        members: Vec<StructureMember>,
    },
    ModifiedType {
        modifier: Modifier,
        inner: TypeId,
    },
}
