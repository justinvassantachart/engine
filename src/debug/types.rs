use crate::debug::dwarf::{Die, DieReference, Dwarf, R};
use ::std::rc::Rc;

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
    /// Equivalent to an rvalue reference in C++
    Temporary,
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

    pub fn declaration(&self) -> Option<&TypeDeclaration> {
        self.graph.get(self.root)
    }
}

impl TypeGraph {
    pub fn new(dwarf: &Dwarf) -> TypeGraph {
        let mut types = HashMap::new();
        for unit in dwarf.units() {
            if let Some(root) = unit.root(dwarf) {
                walk_die(&root, &mut types);
            }
        }
        TypeGraph { types }
    }

    pub fn get(&self, id: TypeId) -> Option<&TypeDeclaration> {
        self.types.get(&id)
    }
}

fn walk_die(die: &Die<'_>, types: &mut HashMap<TypeId, TypeDeclaration>) {
    if let Some(decl) = parse_type_declaration(die) {
        types.insert(die.die_ref(), decl);
    }
    die.for_each_child(|child| walk_die(&child, types));
}

fn parse_type_declaration(die: &Die<'_>) -> Option<TypeDeclaration> {
    match die.tag() {
        gimli::DW_TAG_base_type => Some(TypeDeclaration::Scalar {
            name: die.name().unwrap_or_default(),
            byte_size: u64_attr(die, gimli::DW_AT_byte_size)?,
            encoding: match die.attr_value(gimli::DW_AT_encoding)? {
                gimli::AttributeValue::Encoding(e) => e,
                _ => return None,
            },
        }),
        gimli::DW_TAG_pointer_type => Some(TypeDeclaration::Referential {
            target: die.type_ref()?,
            kind: ReferenceKind::Pointer,
        }),
        gimli::DW_TAG_reference_type => Some(TypeDeclaration::Referential {
            target: die.type_ref()?,
            kind: ReferenceKind::Reference,
        }),
        gimli::DW_TAG_rvalue_reference_type => Some(TypeDeclaration::Referential {
            target: die.type_ref()?,
            kind: ReferenceKind::Temporary,
        }),
        gimli::DW_TAG_typedef => Some(TypeDeclaration::ModifiedType {
            modifier: Modifier::TypeDef,
            inner: die.type_ref()?,
        }),
        gimli::DW_TAG_const_type => Some(TypeDeclaration::ModifiedType {
            modifier: Modifier::Const,
            inner: die.type_ref()?,
        }),
        gimli::DW_TAG_volatile_type => Some(TypeDeclaration::ModifiedType {
            modifier: Modifier::Volatile,
            inner: die.type_ref()?,
        }),
        gimli::DW_TAG_atomic_type => Some(TypeDeclaration::ModifiedType {
            modifier: Modifier::Atomic,
            inner: die.type_ref()?,
        }),
        gimli::DW_TAG_restrict_type => Some(TypeDeclaration::ModifiedType {
            modifier: Modifier::Restrict,
            inner: die.type_ref()?,
        }),
        gimli::DW_TAG_array_type => {
            let element_type = die.type_ref()?;
            let (lower_bound, upper_bound) = die
                .find_children(|c| {
                    (c.tag() == gimli::DW_TAG_subrange_type).then(|| parse_subrange(&c))
                })
                .unwrap_or((ArrayBound::Count(0), None));
            Some(TypeDeclaration::Array {
                byte_size: u64_attr(die, gimli::DW_AT_byte_size),
                element_type,
                lower_bound,
                upper_bound,
            })
        }
        gimli::DW_TAG_structure_type | gimli::DW_TAG_union_type | gimli::DW_TAG_class_type => {
            Some(TypeDeclaration::Structure {
                name: die.name(),
                byte_size: u64_attr(die, gimli::DW_AT_byte_size)?,
                members: die.collect_children(parse_member),
            })
        }
        _ => None,
    }
}

fn parse_member(die: Die<'_>) -> Option<StructureMember> {
    if die.tag() != gimli::DW_TAG_member {
        return None;
    }
    let ty = die.type_ref()?;
    let location = die
        .attr_value(gimli::DW_AT_data_member_location)
        .and_then(|v| match v {
            gimli::AttributeValue::Udata(u) => Some(MemberLocation::Offset(u as i64)),
            gimli::AttributeValue::Sdata(s) => Some(MemberLocation::Offset(s)),
            gimli::AttributeValue::Exprloc(e) => Some(MemberLocation::Expr(e)),
            _ => None,
        });
    Some(StructureMember {
        location,
        name: die.name(),
        ty,
    })
}

fn parse_subrange(die: &Die<'_>) -> (ArrayBound, Option<ArrayBound>) {
    let lower = die
        .attr_value(gimli::DW_AT_lower_bound)
        .and_then(array_bound)
        .unwrap_or(ArrayBound::Count(0));
    let upper = die
        .attr_value(gimli::DW_AT_upper_bound)
        .and_then(array_bound)
        .or_else(|| die.attr_value(gimli::DW_AT_count).and_then(array_bound));
    (lower, upper)
}

fn array_bound(value: gimli::AttributeValue<R>) -> Option<ArrayBound> {
    match value {
        gimli::AttributeValue::Udata(u) => Some(ArrayBound::Count(u as i64)),
        gimli::AttributeValue::Sdata(s) => Some(ArrayBound::Count(s)),
        gimli::AttributeValue::Exprloc(e) => Some(ArrayBound::Expr(e)),
        _ => None,
    }
}

fn u64_attr(die: &Die<'_>, name: gimli::DwAt) -> Option<u64> {
    match die.attr_value(name)? {
        gimli::AttributeValue::Udata(v) => Some(v),
        gimli::AttributeValue::Sdata(s) if s >= 0 => Some(s as u64),
        _ => None,
    }
}
