use super::Debugger;
use crate::debug::Variable;

/// Provides custom expansion for a [Variable].
///
/// Implement this trait to provide custom formatting for variable children
/// and/or variable values.
///
/// The first registered provider whose [`matches`](Self::matches) returns
/// `true` for a variable wins; its [`children`](Self::children) result replaces
/// the default structure/pointer expansion. Providers that only need to alter
/// matching can rely on the default `children` implementation, which yields the
/// raw structural view.
pub trait VariableFormatter {
    /// Performs a quick match on a [Variable] to see if this formatter can
    /// format the given variable.
    ///
    /// [children](Self::children) and [display](Self::display) will not be
    /// invoked on this value unless [matches](Self::matches) returns `true`.
    ///
    /// This function is meant to act as a quick check that the variable *might*
    /// be formatted by this formatter, before executing any expensive
    /// logic that may introspect the variable's actual contents or sub-variables.
    fn matches(&self, value: &Variable) -> bool;

    /// Provides the children for a [Variable].
    ///
    /// The first matching formatter who returns a non-[None] value from
    /// [children](Self::children) wins and replaces the default expansion logic.
    ///
    /// In order to handle errors, if this method returns [None], matching will
    /// proceed with the next registered provider, or the default one if none exist.
    fn children(&self, value: &Variable) -> Option<Vec<Variable>> {
        None
    }

    /// Renders the value for a [Variable].
    ///
    /// The first matching formatter who returns a non-[None] value from
    /// [display](Self::display) wins and replaces the default expansion logic.
    ///
    /// In order to handle errors, if this method returns [None], matching will
    /// proceed with the next registered provider, or the default one if none exist.
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
