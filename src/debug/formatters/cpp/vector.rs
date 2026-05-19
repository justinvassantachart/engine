//! `std::vector` formatter.

use std::ops::Range;

use anyhow::{Context, Result};

use crate::debug::Variable;
use crate::debug::formatters::{ChildCounts, VariableFormatter};

pub struct StdVectorFormatter;

impl StdVectorFormatter {
    fn data(value: &Variable) -> Result<(Variable, usize)> {
        let begin = value
            .child_with_name("__begin_")
            .context("No child named '__begin_'")?;
        let end = value
            .child_with_name("__end_")
            .context("No child named '__end_'")?;

        let begin_addr = begin
            .pointer_value()
            .context("std::vector __begin_ is unavailable")?;
        let end_addr = end
            .pointer_value()
            .context("std::vector __end_ is unavailable")?;

        let elem_size = begin
            .ty()
            .pointee()
            .and_then(|ty| ty.byte_size())
            .context("std::vector element size is unavailable")?;
        if elem_size == 0 {
            return Ok((begin.clone(), 0));
        }

        let bytes = end_addr.0.saturating_sub(begin_addr.0);
        Ok((begin.clone(), (bytes / elem_size) as usize))
    }
}

impl VariableFormatter for StdVectorFormatter {
    fn matches(&self, value: &Variable) -> bool {
        let name = value.ty().name();
        value.ty().ns().matches("std")
            && (name == "std::vector" || name.starts_with("std::vector<"))
    }

    fn display(&self, value: &Variable) -> Result<String> {
        value.display()
    }

    fn num_children(&self, value: &Variable) -> Result<ChildCounts> {
        let (_, count) = Self::data(value)?;
        Ok(ChildCounts::indexed(count))
    }

    fn indexed_children(&self, value: &Variable, range: Range<usize>) -> Result<Vec<Variable>> {
        let (begin, count) = Self::data(value)?;
        let start = range.start.min(count);
        let end = range.end.min(count);
        begin.indexed_children(start..end)
    }

    fn named_children(&self, _value: &Variable, _range: Range<usize>) -> Result<Vec<Variable>> {
        Ok(Vec::new())
    }
}
