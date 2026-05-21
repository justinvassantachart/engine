use std::ops::Range;

use anyhow::{Result, bail};

use crate::debug::formatters::{ChildCounts, VariableFormatter};
use crate::debug::{ReferenceKind, Type, TypeDeclaration, Variable};

pub struct ReferentialFormatter;

impl VariableFormatter for ReferentialFormatter {
    fn matches(&self, ty: &Type) -> bool {
        matches!(ty.resolved(), Some(TypeDeclaration::Referential { .. }))
    }

    fn display(&self, value: &Variable) -> Result<String> {
        match value.ty().resolved() {
            Some(TypeDeclaration::Referential { target, kind, .. }) => match kind {
                ReferenceKind::Pointer => Ok(match value.address() {
                    Some(addr) => addr.to_string(),
                    None => "<unavailable>".into(),
                }),
                ReferenceKind::Reference | ReferenceKind::Temporary => {
                    return value
                        .child_at_offset(0)
                        .with_type(&value.ty().child(*target))
                        .display();
                }
            },
            _ => bail!("Cannot format non-referential value"),
        }
    }

    fn num_children(&self, value: &Variable) -> Result<ChildCounts> {
        map_reference(
            value,
            |_referent| Ok(ChildCounts::named(1)),
            |referent| referent.num_children(),
        )
    }

    fn indexed_children(&self, value: &Variable, range: Range<usize>) -> Result<Vec<Variable>> {
        map_reference(
            value,
            |_referent| Ok(Vec::default()),
            |referent| referent.indexed_children(range),
        )
    }

    fn named_children(&self, value: &Variable, range: Range<usize>) -> Result<Vec<Variable>> {
        let contains_first = range.contains(&0);
        map_reference(
            value,
            |referent| {
                Ok(if contains_first {
                    vec![referent.with_name(&format!("*{}", value.name()))]
                } else {
                    Vec::default()
                })
            },
            |referent| referent.named_children(range),
        )
    }
}

fn map_reference<T>(
    value: &Variable,
    scalar: impl FnOnce(Variable) -> Result<T>,
    structure: impl FnOnce(Variable) -> Result<T>,
) -> Result<T>
where
    T: Default,
{
    match value.ty().resolved() {
        Some(TypeDeclaration::Referential { target, kind, .. }) => {
            let referent = value
                .child_at_offset(0)
                .with_type(&value.ty().child(*target));

            match kind {
                ReferenceKind::Pointer => {
                    let Some(addr) = value.pointer_value() else {
                        return Ok(T::default());
                    };

                    if addr.is_null() {
                        return Ok(T::default());
                    };

                    match value.ty().child(*target).resolved() {
                        Some(TypeDeclaration::Scalar { .. }) => scalar(referent),
                        _ => structure(referent),
                    }
                }
                ReferenceKind::Reference | ReferenceKind::Temporary => return structure(referent),
            }
        }
        _ => bail!("Cannot format non-referential value"),
    }
}
