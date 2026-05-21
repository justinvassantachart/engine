use std::ops::Range;

use anyhow::Result;

use crate::debug::{Type, Variable};

pub mod cpp;
pub mod default;

#[derive(Clone, Copy, Debug, Default)]
pub struct ChildCounts {
    /// The number of indexed children the variable has.
    ///
    /// Indexed children usually correspond to elements in container data types and
    /// usually have names like `[0]`, `[1]`, `[2]`, and so on.
    pub indexed: usize,

    /// The number of named children the variable has.
    ///
    /// Named children usually correspond to members in structured data types and have
    /// names corresponding to the names of those members.
    pub named: usize,
}

impl ChildCounts {
    /// Children counts for a variable with `indexed` indexed children and `named` named children.
    pub fn mixed(indexed: usize, named: usize) -> Self {
        Self { indexed, named }
    }

    /// Children counts for a variable with `count` indexed children and no named children.
    pub fn indexed(count: usize) -> Self {
        Self {
            indexed: count,
            named: 0,
        }
    }

    /// Children counts for a variable with `count` named children and no indexed children.
    pub fn named(count: usize) -> Self {
        Self {
            indexed: 0,
            named: count,
        }
    }

    /// Children counts for a variable with no children.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn total(&self) -> usize {
        self.indexed + self.named
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

/// Provides custom presentation and expansion for a [Variable].
///
/// The first registered formatter whose [matches](Self::matches) method returns `true`
/// owns the whole formatted view for that variable. It provides the display text,
/// child counts, and child slices so a client can ask for a small range without
/// materializing a large container.
pub trait VariableFormatter {
    /// Returns whether this formatter can present a value with type `ty`.
    fn matches(&self, ty: &Type) -> bool;

    /// Renders the value for display in the variables view.
    fn display(&self, value: &Variable) -> Result<String>;

    /// Returns how many formatted children `value` has.
    fn num_children(&self, value: &Variable) -> Result<ChildCounts>;

    /// Returns indexed children in `range`.
    fn indexed_children(&self, value: &Variable, range: Range<usize>) -> Result<Vec<Variable>>;

    /// Returns named children in `range`.
    fn named_children(&self, value: &Variable, range: Range<usize>) -> Result<Vec<Variable>>;
}
