//! `std::string` formatter.

use std::ops::Range;

use anyhow::Result;

use crate::debug::formatters::{ChildCounts, VariableFormatter};
use crate::debug::{Type, Variable};

pub struct StdStringFormatter;

impl VariableFormatter for StdStringFormatter {
    fn matches(&self, ty: &Type) -> bool {
        let name = ty.name();
        ty.ns().matches("std") && (name == "std::string" || name.starts_with("std::string"))
    }

    fn display(&self, _value: &Variable) -> Result<String> {
        anyhow::bail!("not implemented")
    }

    fn num_children(&self, _value: &Variable) -> Result<ChildCounts> {
        anyhow::bail!("not implemented")
    }

    fn indexed_children(&self, _value: &Variable, _range: Range<usize>) -> Result<Vec<Variable>> {
        anyhow::bail!("not implemented")
    }

    fn named_children(&self, _value: &Variable, _range: Range<usize>) -> Result<Vec<Variable>> {
        anyhow::bail!("not implemented")
    }
}
