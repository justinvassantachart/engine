//! `std::vector` formatter.

use std::ops::Range;

use anyhow::{Context, Result};

use crate::debug::formatters::default::StructureFormatter;
use crate::debug::formatters::{ChildCounts, VariableFormatter};
use crate::debug::{Type, Variable};

pub struct StdVectorFormatter;

struct VectorData {
    begin: Variable,
    count: usize,
    elem_ty: Type,
    elem_size: usize,
}

impl StdVectorFormatter {
    fn data(value: &Variable) -> Result<VectorData> {
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

        let elem_ty = begin
            .ty()
            .pointee()
            .context("std::vector element type is unavailable")?;
        let elem_size = elem_ty
            .byte_size()
            .context("std::vector element size is unavailable")? as usize;

        if elem_size == 0 {
            return Ok(VectorData {
                begin: begin.clone(),
                count: 0,
                elem_ty,
                elem_size: 0,
            });
        }

        let bytes = end_addr.0.saturating_sub(begin_addr.0);
        Ok(VectorData {
            begin: begin.clone(),
            count: bytes as usize / elem_size,
            elem_ty,
            elem_size,
        })
    }
}

impl VariableFormatter for StdVectorFormatter {
    fn matches(&self, ty: &Type) -> bool {
        let name = ty.name();
        ty.ns().matches("std") && (name == "std::vector" || name.starts_with("std::vector<"))
    }

    fn display(&self, value: &Variable) -> Result<String> {
        StructureFormatter.display(value)
    }

    fn num_children(&self, value: &Variable) -> Result<ChildCounts> {
        let data = Self::data(value)?;
        Ok(ChildCounts::indexed(data.count))
    }

    fn indexed_children(&self, value: &Variable, range: Range<usize>) -> Result<Vec<Variable>> {
        let data = Self::data(value)?;
        let start = range.start.min(data.count);
        let end = range.end.min(data.count);
        if data.elem_size == 0 {
            return Ok(Vec::new());
        }

        let mut result = Vec::with_capacity(end.saturating_sub(start));
        for i in start..end {
            result.push(
                data.begin
                    .child_at_offset(i * data.elem_size)
                    .with_name(&format!("[{i}]"))
                    .with_type(&data.elem_ty),
            );
        }
        Ok(result)
    }

    fn named_children(&self, _value: &Variable, _range: Range<usize>) -> Result<Vec<Variable>> {
        Ok(Vec::new())
    }
}
