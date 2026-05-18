//! `std::string` formatter.

use std::ops::Range;

use anyhow::Result;

use crate::debug::Variable;
use crate::debug::formatters::{ChildCounts, VariableFormatter};

pub struct StdStringFormatter;

impl VariableFormatter for StdStringFormatter {
    fn matches(&self, value: &Variable) -> bool {
        let name = value.ty().name();
        value.ty().ns().matches("std") && (name == "std::string" || name.starts_with("std::string"))
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
