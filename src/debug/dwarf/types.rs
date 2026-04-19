use crate::debug::dwarf::{DieReference, Dwarf};
use ::std::rc::Rc;

use super::R;
use std::collections::HashMap;

type TypeId = DieReference;

pub struct Type {
    root: TypeId,
    graph: Rc<TypeGraph>,
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

impl Type {
    pub fn new(root: TypeId, graph: Rc<TypeGraph>) -> Self {
        Self { root, graph }
    }
}

pub fn parse_graph(dwarf: &Dwarf) -> Option<TypeGraph> {
    // iter through dwarf sections
    // populate tg (TypeGraph) with DieLocation (TypeId)
    None
}
