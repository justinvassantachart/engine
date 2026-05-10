use crate::debug::dwarf::{Die, DieReference, Dwarf, R};
use ::std::rc::Rc;

use std::collections::HashMap;

pub type TypeId = DieReference;

#[derive(Clone, Debug, Default)]
pub struct NamespaceHierarchy(Vec<String>);

impl NamespaceHierarchy {
    pub fn push(&mut self, name: String) {
        self.0.push(name);
    }

    pub fn pop(&mut self) {
        self.0.pop();
    }

    pub fn qualify(&self, name: &str) -> String {
        let qualified = if self.0.is_empty() {
            name.to_string()
        } else {
            format!("{}::{name}", self.0.join("::"))
        };

        // Note: qualified `std` library names have the inline marker `std::__2`.
        // TODO: This is a hack, we should really parse out all template parameters
        qualified.replace("std::__2::", "std::")
    }

    pub fn matches(&self, target: &str) -> bool {
        if target.is_empty() {
            return false;
        }
        let parts: Vec<&str> = target.split("::").collect();
        self.0.len() >= parts.len()
            && self.0.iter().zip(&parts).all(|(a, b)| a == b)
    }
}

#[derive(Clone)]
pub struct Type {
    root: TypeId,
    graph: Rc<TypeGraph>,
}

pub struct TypeGraph {
    types: HashMap<TypeId, TypeDeclaration>,
}

/// Represents a value that can either hold a constant integer value
/// or be encoded as an expression.
#[derive(Clone, Debug)]
pub enum Value {
    Constant(i64),
    Expr(gimli::Expression<R>),
}

#[derive(Clone, Debug)]
pub struct StructureMember {
    pub location: Option<Value>,
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
pub enum TypeDeclaration {
    Scalar {
        ns: NamespaceHierarchy,
        name: String,
        byte_size: u64,
        encoding: gimli::DwAte,
    },
    Array {
        ns: NamespaceHierarchy,
        byte_size: Option<u64>,
        element_type: TypeId,
        lower_bound: Value,
        upper_bound: Option<Value>,
    },
    Referential {
        ns: NamespaceHierarchy,
        target: TypeId,
        kind: ReferenceKind,
    },
    Structure {
        ns: NamespaceHierarchy,
        name: Option<String>,
        byte_size: u64,
        members: Vec<StructureMember>,
    },
    ModifiedType {
        ns: NamespaceHierarchy,
        name: Option<String>,
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

    /// Returns a [`Type`] over the same graph, rooted at `id`.
    pub fn child(&self, id: TypeId) -> Type {
        Type {
            root: id,
            graph: self.graph.clone(),
        }
    }

    /// Walks past `typedef`/cv-qualifier modifiers and returns the underlying declaration.
    pub fn resolved(&self) -> Option<&TypeDeclaration> {
        let mut current = self.declaration()?;
        loop {
            match current {
                TypeDeclaration::ModifiedType { inner, .. } => {
                    current = self.graph.get(*inner)?;
                }
                _ => return Some(current),
            }
        }
    }

    /// Human-readable name of this type (e.g. `int`, `Point`, `int*`).
    pub fn name(&self) -> String {
        decl_name(self.declaration(), &self.graph)
    }

    /// Size in bytes of this type, or `None` if unknown.
    pub fn byte_size(&self) -> Option<u64> {
        match self.resolved()? {
            TypeDeclaration::Scalar { byte_size, .. } => Some(*byte_size),
            TypeDeclaration::Structure { byte_size, .. } => Some(*byte_size),
            TypeDeclaration::Array { byte_size, .. } => *byte_size,
            // Pointers/references are wasm32 — 4 bytes
            // TODO: Use the unit address size
            TypeDeclaration::Referential { .. } => Some(4),
            _ => None,
        }
    }

    pub fn ns(&self) -> NamespaceHierarchy {
        match self.resolved() {
            Some(TypeDeclaration::Scalar { ns, .. })
            | Some(TypeDeclaration::Array { ns, .. })
            | Some(TypeDeclaration::Referential { ns, .. })
            | Some(TypeDeclaration::Structure { ns, .. })
            | Some(TypeDeclaration::ModifiedType { ns, .. }) => ns.clone(),
            None => NamespaceHierarchy::default(),
        }
    }
}

fn decl_name(decl: Option<&TypeDeclaration>, graph: &TypeGraph) -> String {
    let Some(decl) = decl else {
        return "<unknown>".to_string();
    };
    match decl {
        TypeDeclaration::Scalar { name, ns, .. } => ns.qualify(name),
        TypeDeclaration::Structure { name, ns, .. } => name
            .as_deref()
            .map(|name| ns.qualify(name))
            .unwrap_or_else(|| "<anonymous>".to_string()),
        TypeDeclaration::Referential { target, kind, .. } => {
            let inner = decl_name(graph.get(*target), graph);
            match kind {
                ReferenceKind::Pointer => format!("{inner}*"),
                ReferenceKind::Reference => format!("{inner}&"),
                ReferenceKind::Temporary => format!("{inner}&&"),
            }
        }
        TypeDeclaration::Array {
            element_type,
            lower_bound,
            upper_bound,
            ..
        } => {
            let elem = decl_name(graph.get(*element_type), graph);
            let count = match (lower_bound, upper_bound) {
                (Value::Constant(lo), Some(Value::Constant(hi))) => Some(hi - lo + 1),
                _ => None,
            };
            match count {
                Some(c) => format!("{elem}[{c}]"),
                None => format!("{elem}[]"),
            }
        }
        TypeDeclaration::ModifiedType {
            name,
            ns,
            modifier,
            inner,
        } => {
            if let Some(name) = name {
                return ns.qualify(name);
            }
            let inner_name = decl_name(graph.get(*inner), graph);
            match modifier {
                Modifier::TypeDef => inner_name,
                Modifier::Const => format!("const {inner_name}"),
                Modifier::Volatile => format!("volatile {inner_name}"),
                Modifier::Atomic => format!("_Atomic {inner_name}"),
                Modifier::Restrict => format!("restrict {inner_name}"),
            }
        }
    }
}

impl TypeGraph {
    pub fn new(dwarf: &Dwarf) -> TypeGraph {
        let mut types = HashMap::new();
        for unit in dwarf.units() {
            if let Some(root) = unit.root(dwarf) {
                walk_die(&root, &mut types, &mut NamespaceHierarchy::default());
            }
        }
        TypeGraph { types }
    }

    pub fn get(&self, id: TypeId) -> Option<&TypeDeclaration> {
        self.types.get(&id)
    }
}

fn walk_die(
    die: &Die<'_>,
    types: &mut HashMap<TypeId, TypeDeclaration>,
    ns: &mut NamespaceHierarchy,
) {
    let ns_part = parse_namespace_component(die);
    let has_ns = ns_part.is_some();

    if let Some(ns_part) = ns_part {
        ns.push(ns_part);
    }

    if let Some(decl) = parse_type_declaration(die, ns) {
        types.insert(die.die_ref(), decl);
    }
    die.for_each_child(|child| walk_die(&child, types, ns));

    if has_ns {
        ns.pop();
    }
}

fn parse_namespace_component(die: &Die<'_>) -> Option<String> {
    if die.tag() == gimli::DW_TAG_namespace {
        // If DW_AT_export_symbols is set to `true`, this represents
        // an inline namespace such as the `__2` in `std::__2`.
        // These shouldn't be included in the namespace chain since they are
        // semantically ignored.
        let exported = matches!(
            die.attr_value(gimli::DW_AT_export_symbols),
            Some(gimli::AttributeValue::Flag(true))
        );
        if exported { None } else { die.name() }
    } else {
        None
    }
}

fn parse_type_declaration(die: &Die<'_>, ns: &NamespaceHierarchy) -> Option<TypeDeclaration> {
    let ns = ns.clone();
    match die.tag() {
        gimli::DW_TAG_base_type => Some(TypeDeclaration::Scalar {
            name: die.name().unwrap_or_default(),
            ns,
            byte_size: u64_attr(die, gimli::DW_AT_byte_size)?,
            encoding: match die.attr_value(gimli::DW_AT_encoding)? {
                gimli::AttributeValue::Encoding(e) => e,
                _ => return None,
            },
        }),
        gimli::DW_TAG_pointer_type => Some(TypeDeclaration::Referential {
            ns,
            target: die.type_ref()?,
            kind: ReferenceKind::Pointer,
        }),
        gimli::DW_TAG_reference_type => Some(TypeDeclaration::Referential {
            ns,
            target: die.type_ref()?,
            kind: ReferenceKind::Reference,
        }),
        gimli::DW_TAG_rvalue_reference_type => Some(TypeDeclaration::Referential {
            ns,
            target: die.type_ref()?,
            kind: ReferenceKind::Temporary,
        }),
        gimli::DW_TAG_typedef => Some(TypeDeclaration::ModifiedType {
            name: die.name(),
            ns,
            modifier: Modifier::TypeDef,
            inner: die.type_ref()?,
        }),
        gimli::DW_TAG_const_type => Some(TypeDeclaration::ModifiedType {
            name: die.name(),
            ns,
            modifier: Modifier::Const,
            inner: die.type_ref()?,
        }),
        gimli::DW_TAG_volatile_type => Some(TypeDeclaration::ModifiedType {
            name: die.name(),
            ns,
            modifier: Modifier::Volatile,
            inner: die.type_ref()?,
        }),
        gimli::DW_TAG_atomic_type => Some(TypeDeclaration::ModifiedType {
            name: die.name(),
            ns,
            modifier: Modifier::Atomic,
            inner: die.type_ref()?,
        }),
        gimli::DW_TAG_restrict_type => Some(TypeDeclaration::ModifiedType {
            name: die.name(),
            ns,
            modifier: Modifier::Restrict,
            inner: die.type_ref()?,
        }),
        gimli::DW_TAG_array_type => {
            let element_type = die.type_ref()?;
            let (lower_bound, upper_bound) = die
                .find_children(|c| {
                    (c.tag() == gimli::DW_TAG_subrange_type).then(|| parse_subrange(&c))
                })
                .unwrap_or((Value::Constant(0), None));
            Some(TypeDeclaration::Array {
                ns,
                byte_size: u64_attr(die, gimli::DW_AT_byte_size),
                element_type,
                lower_bound,
                upper_bound,
            })
        }
        gimli::DW_TAG_structure_type | gimli::DW_TAG_union_type | gimli::DW_TAG_class_type => {
            Some(TypeDeclaration::Structure {
                name: die.name(),
                ns,
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
            gimli::AttributeValue::Udata(u) => Some(Value::Constant(u as i64)),
            gimli::AttributeValue::Sdata(s) => Some(Value::Constant(s)),
            gimli::AttributeValue::Exprloc(e) => Some(Value::Expr(e)),
            _ => None,
        });
    Some(StructureMember {
        location,
        name: die.name(),
        ty,
    })
}

fn parse_subrange(die: &Die<'_>) -> (Value, Option<Value>) {
    let lower = die
        .attr_value(gimli::DW_AT_lower_bound)
        .and_then(array_bound)
        .unwrap_or(Value::Constant(0));
    let upper = die
        .attr_value(gimli::DW_AT_upper_bound)
        .and_then(array_bound)
        .or_else(|| die.attr_value(gimli::DW_AT_count).and_then(array_bound));
    (lower, upper)
}

fn array_bound(value: gimli::AttributeValue<R>) -> Option<Value> {
    match value {
        gimli::AttributeValue::Udata(u) => Some(Value::Constant(u as i64)),
        gimli::AttributeValue::Sdata(s) => Some(Value::Constant(s)),
        gimli::AttributeValue::Exprloc(e) => Some(Value::Expr(e)),
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
