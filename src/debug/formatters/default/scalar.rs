use std::ops::Range;

use anyhow::{Result, bail};

use crate::debug::formatters::{ChildCounts, VariableFormatter};
use crate::debug::{Type, TypeDeclaration, Variable};

pub struct ScalarFormatter;

impl VariableFormatter for ScalarFormatter {
    fn matches(&self, ty: &Type) -> bool {
        matches!(ty.resolved(), Some(TypeDeclaration::Scalar { .. }))
    }

    fn display(&self, value: &Variable) -> Result<String> {
        match value.ty().resolved() {
            Some(TypeDeclaration::Scalar {
                byte_size,
                encoding,
                ..
            }) => {
                let Some(bytes) = value.read(*byte_size as usize) else {
                    return Ok("<unavailable>".into());
                };
                Ok(format_scalar(&bytes, *encoding))
            }
            _ => bail!("Cannot format non-scalar value"),
        }
    }

    fn num_children(&self, _value: &Variable) -> Result<ChildCounts> {
        Ok(ChildCounts::none())
    }

    fn indexed_children(&self, _value: &Variable, _range: Range<usize>) -> Result<Vec<Variable>> {
        Ok(Vec::new())
    }

    fn named_children(&self, _value: &Variable, _range: Range<usize>) -> Result<Vec<Variable>> {
        Ok(Vec::new())
    }
}

fn format_scalar(bytes: &[u8], encoding: gimli::DwAte) -> String {
    let size = bytes.len();
    match encoding {
        gimli::DW_ATE_signed | gimli::DW_ATE_signed_char => match size {
            1 => (bytes[0] as i8).to_string(),
            2 => i16::from_le_bytes(bytes.try_into().unwrap()).to_string(),
            4 => i32::from_le_bytes(bytes.try_into().unwrap()).to_string(),
            8 => i64::from_le_bytes(bytes.try_into().unwrap()).to_string(),
            _ => "<unsupported size>".into(),
        },
        gimli::DW_ATE_unsigned | gimli::DW_ATE_unsigned_char => match size {
            1 => bytes[0].to_string(),
            2 => u16::from_le_bytes(bytes.try_into().unwrap()).to_string(),
            4 => u32::from_le_bytes(bytes.try_into().unwrap()).to_string(),
            8 => u64::from_le_bytes(bytes.try_into().unwrap()).to_string(),
            _ => "<unsupported size>".into(),
        },
        gimli::DW_ATE_boolean => {
            let v = bytes.iter().any(|&b| b != 0);
            (if v { "true" } else { "false" }).into()
        }
        gimli::DW_ATE_float => match size {
            4 => f32::from_le_bytes(bytes.try_into().unwrap()).to_string(),
            8 => f64::from_le_bytes(bytes.try_into().unwrap()).to_string(),
            _ => "<unsupported float size>".into(),
        },
        gimli::DW_ATE_UTF | gimli::DW_ATE_ASCII => match size {
            1 => format!("{:?}", bytes[0] as char),
            _ => "<unsupported char size>".into(),
        },
        _ => "<unsupported encoding>".into(),
    }
}
