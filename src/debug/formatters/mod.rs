use std::ops::Range;

use super::Debugger;
use crate::debug::Variable;

pub struct ChildCounts {
    /// The number of indexed children the variable has.
    ///
    /// Indexed children usually correspond to container data types and
    /// usually have names like `[0]`, `[1]`, `[2]`, and so on.
    pub indexed: usize,

    /// The number of named children the variable has.
    ///
    /// Named children usually correspond to structured types and have
    /// names corresponding to the member names.
    pub named: usize,
}

/// Provides custom expansion for a [Variable].
///
/// Implement this trait to provide custom formatting for variable children
/// and/or variable values.
pub trait VariableFormatter {
    /// Performs a quick match on a [Variable] to see if this formatter can
    /// format the given variable.
    ///
    /// This function is meant to act as a quick check that the variable *might*
    /// be formatted by this formatter, before executing any expensive
    /// logic that may introspect the variable's actual contents or sub-variables.
    fn matches(&self, value: &Variable) -> bool;

    /// Computes the number of children that a variable has.
    ///
    /// Returns [None] if this formatter cannot provide children for this node.
    /// If the return value is [Some], the debugger may proceed with calling
    /// [indexed_children](Self::indexed_children) and [named_children](Self::named_children)
    /// for this variable.
    #[allow(unused)]
    fn num_children(&self, value: &Variable) -> Option<ChildCounts> {
        None
    }

    /// Provides the indexed children for a [Variable] within this range.
    #[allow(unused)]
    fn indexed_children(&self, value: &Variable, range: Range<usize>) -> Vec<Variable> {
        Vec::new()
    }

    /// Provides the named children for a [Variable] within this range.
    #[allow(unused)]
    fn named_children(&self, value: &Variable, range: Range<usize>) -> Vec<Variable> {
        Vec::new()
    }

    /// Renders the value for a [Variable].
    ///
    /// The first matching formatter who returns a non-[None] value from
    /// [display](Self::display) wins and replaces the default expansion logic.
    ///
    /// In order to handle errors, if this method returns [None], matching will
    /// proceed with the next registered provider, or the default one if none exist.
    #[allow(unused)]
    fn display(&self, value: &Variable) -> Option<String> {
        None
    }
}

/// Registers the built-in formatters on `dbg`.
pub fn register_defaults(_dbg: &mut Debugger) {}

// Reusable for various formatter that follow the same access pattern
pub trait VariableSliceExt {
    fn find(&self, name: &str) -> Option<&Variable>;
}

impl VariableSliceExt for [Variable] {
    fn find(&self, name: &str) -> Option<&Variable> {
        self.iter().find(|v| v.name() == name)
    }
}
