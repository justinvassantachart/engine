//! libc++ `std::vector<T>` synthetic children.

use crate::debug::{Debugger, Variable, VariableProvider};

pub struct StdVectorProvider;

impl VariableProvider for StdVectorProvider {
    fn matches(&self, value: &Variable) -> bool {
        value.ty().ns().matches("std") && value.ty().name() == "vector"
    }

    fn children(&self, value: &Variable, dbg: &Debugger) -> Vec<Variable> {
        let vec_children = value.children(&dbg.info);
        let begin = vec_children.find("__begin_");
        let end = vec_children.find("__end_");

        let begin_addr = begin.addr_value(&dbg.info);
        let end_addr = end.addr_value(&dbg.info);

        let elem_size = begin.ty().child().byte_size().unwrap_or(1);
        let count = end_addr.saturating_sub(begin_addr) / elem_size;
        begin.indexed_children(&dbg.info, 0, count as usize)
    }
}
