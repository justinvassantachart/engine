use crate::{
    debug::dwarf::{Die, Type, Visit},
    types::{DebugInfo, GlobalAddress},
};
use gimli::read::Expression;

use super::R;

/// Gets all live variables at the given PC.
///
/// `die` should be a reference to a subprogram,
/// and `pc` should be an offset within the wasm code segment.
pub fn get_variables<'a>(die: &Die<'a>, pc: GlobalAddress) -> Vec<Die<'a>> {
    assert!(
        die.tag() == gimli::DW_TAG_subprogram,
        "get_variables requires subprogram die"
    );

    let mut result: Vec<Die<'a>> = vec![];

    die.traverse(|child| {
        let tag = child.tag();

        match tag {
            gimli::DW_TAG_formal_parameter | gimli::DW_TAG_local_variable => {
                result.push(child);
            }

            _ => {
                if let Some(low) = child.low_pc()
                    && pc < low
                {
                    return Visit::SkipChildren;
                }

                if let Some(high) = child.high_pc()
                    && pc >= high
                {
                    return Visit::SkipChildren;
                }
            }
        }

        Visit::Continue
    });

    result
}

/// Gets the location expression for a variable at the given PC
pub fn get_location(die: &Die<'_>, pc: GlobalAddress) -> Option<Expression<R>> {
    die.expression(gimli::DW_AT_location, pc)
}

pub struct Value {
    inner: Vec<gimli::Piece<R>>,
    ty: Type,
}

impl Value {
    pub fn new(pieces: Vec<gimli::Piece<R>>, ty: Type) -> Self {
        Self { inner: pieces, ty }
    }

    /// initial idea: look at the pieces
    pub fn address(&self) -> Option<u64> {
        None
    }

    /// Need the info to inspect the wasm locactions
    pub fn children(&self, info: &DebugInfo) -> Vec<Value> {
        Vec::default()
    }
}
