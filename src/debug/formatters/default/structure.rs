use std::ops::Range;

use anyhow::{Result, bail};

use crate::debug::formatters::{ChildCounts, VariableFormatter};
use crate::debug::{Type, TypeDeclaration, Value, Variable};

pub struct StructureFormatter;

impl VariableFormatter for StructureFormatter {
    fn matches(&self, ty: &Type) -> bool {
        matches!(ty.resolved(), Some(TypeDeclaration::Structure { .. }))
    }

    fn display(&self, value: &Variable) -> Result<String> {
        match value.ty().resolved() {
            Some(TypeDeclaration::Structure { .. }) => Ok(if let Some(addr) = value.address() {
                format!("@{addr}")
            } else {
                String::default()
            }),
            _ => bail!("Cannot format non-structure value"),
        }
    }

    fn num_children(&self, value: &Variable) -> Result<ChildCounts> {
        match value.ty().resolved() {
            Some(TypeDeclaration::Structure { members, .. }) => Ok(ChildCounts::named(
                members
                    .iter()
                    .filter(|member| member.name.is_some())
                    .count(),
            )),
            _ => bail!("Calling num_children on non-structure value"),
        }
    }

    fn indexed_children(&self, _value: &Variable, _range: Range<usize>) -> Result<Vec<Variable>> {
        Ok(Vec::new())
    }

    fn named_children(&self, value: &Variable, range: Range<usize>) -> Result<Vec<Variable>> {
        match value.ty().resolved() {
            Some(TypeDeclaration::Structure { members, .. }) => {
                let mut out = Vec::with_capacity(members.len());
                for member in members {
                    let Some(name) = member.name.as_deref() else {
                        continue;
                    };

                    let offset = match &member.location {
                        Some(Value::Constant(o)) => *o,
                        None => 0,
                        Some(Value::Expr(_)) => continue,
                    };

                    out.push(
                        value
                            .child_at_offset(offset as usize)
                            .with_name(name)
                            .with_type(&value.ty().child(member.ty)),
                    );
                }
                Ok(out[range].to_vec())
            }
            _ => bail!("Calling named_children on non-structure value"),
        }
    }
}
