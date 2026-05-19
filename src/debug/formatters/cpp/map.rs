//! `std::map` and `std::set` formatters (`std::__tree`).
//!
//! See the corresponding [LLDB formatter code](https://github.com/llvm/llvm-project/blob/main/lldb/source/Plugins/Language/CPlusPlus/LibCxxMap.cpp)
//! for reference.

use std::ops::Range;

use anyhow::{Context, Result};

use crate::debug::{Type, Variable};
use crate::debug::formatters::{ChildCounts, VariableFormatter};

const VALUE_OFFSET: usize = 16;

struct LibcxxTree {
    begin_node: Variable,
    value_ty: Type,
    count: usize,
    ptr_size: usize,
}

impl LibcxxTree {
    fn new(value: &Variable) -> Result<Self> {
        let tree = value
            .child_with_name("__tree_")
            .context("No child named '__tree_'")?;
        let begin_node = tree
            .child_with_name("__begin_node_")
            .context("No child named '__begin_node_'")?;
        let value_ty = tree
            .ty()
            .discard_modifiers()
            .direct_nested_type_with_name("value_type")?
            .discard_modifiers();
        let ptr_size = begin_node
            .debugger()
            .map(|debugger| debugger.pointer_size())
            .unwrap_or(4) as usize;
        Ok(Self {
            begin_node,
            value_ty,
            count: tree
                .child_with_name("__size_")
                .context("No child named '__size_'")?
                .u64_value()
                .context("Could not read __size_")? as usize,
            ptr_size,
        })
    }

    fn indexed_children(&self, range: Range<usize>) -> Result<Vec<Variable>> {
        let end = range.end.min(self.count);
        let mut iter = TreeIter::new(self);
        for _ in 0..range.start {
            if !iter.advance() {
                return Ok(Vec::new());
            }
        }
        let mut out = Vec::with_capacity(end - range.start);
        for index in range.start..end {
            out.push(
                iter.value()
                    .with_name(&format!("[{index}]"))
                    .with_type(&self.value_ty),
            );
            if index + 1 < end && !iter.advance() {
                break;
            }
        }
        Ok(out)
    }
}

struct TreeIter<'a> {
    tree: &'a LibcxxTree,
    /// A `__node_pointer` to the current tree node.
    current: Variable,
    steps: usize,
    done: bool,
}

impl<'a> TreeIter<'a> {
    fn new(tree: &'a LibcxxTree) -> Self {
        Self {
            tree,
            current: tree.begin_node.clone(),
            steps: 0,
            done: false,
        }
    }

    fn left(&self) -> Option<Variable> {
        self.link(0)
    }

    fn right(&self) -> Option<Variable> {
        self.link(self.tree.ptr_size)
    }

    fn parent(&self) -> Option<Variable> {
        self.link(2 * self.tree.ptr_size)
    }

    fn link(&self, offset: usize) -> Option<Variable> {
        let link = self.current.child_at_offset(offset);
        let address = link.pointer_value()?;
        if address.is_null() {
            return None;
        }
        Some(link.child_at_offset(0))
    }

    fn value(&self) -> Variable {
        self.current.child_at_offset(VALUE_OFFSET)
    }

    fn advance(&mut self) -> bool {
        if self.done || self.current.pointer_value().is_none_or(|address| address.is_null()) {
            return false;
        }
        if let Some(right) = self.right() {
            self.current = right;
            while let Some(left) = self.left() {
                self.current = left;
                self.steps += 1;
                if self.steps > self.tree.count {
                    self.done = true;
                    return false;
                }
            }
            return true;
        }
        while !self.is_left_child() {
            let Some(parent) = self.parent() else {
                self.done = true;
                return false;
            };
            self.current = parent;
            self.steps += 1;
            if self.steps > self.tree.count {
                self.done = true;
                return false;
            }
        }
        match self.parent() {
            Some(next) => {
                self.current = next;
                true
            }
            None => {
                self.done = true;
                false
            }
        }
    }

    fn is_left_child(&self) -> bool {
        let here = self.current.address();
        let Some(parent) = self.parent() else {
            return false;
        };
        Self {
            tree: self.tree,
            current: parent,
            steps: 0,
            done: false,
        }
        .left()
        .is_some_and(|left| left.address() == here)
    }
}

fn is_container(value: &Variable, container: &str) -> bool {
    let name = value.ty().name();
    name.starts_with(&format!("std::{container}<")) && !name.contains(">::")
}

pub struct StdMapFormatter;

impl VariableFormatter for StdMapFormatter {
    fn matches(&self, value: &Variable) -> bool {
        is_container(value, "map") || is_container(value, "set")
    }
    fn display(&self, value: &Variable) -> Result<String> {
        value.display()
    }
    fn num_children(&self, value: &Variable) -> Result<ChildCounts> {
        Ok(ChildCounts::indexed(LibcxxTree::new(value)?.count))
    }
    fn indexed_children(&self, value: &Variable, range: Range<usize>) -> Result<Vec<Variable>> {
        LibcxxTree::new(value)?.indexed_children(range)
    }
    fn named_children(&self, _: &Variable, _: Range<usize>) -> Result<Vec<Variable>> {
        Ok(Vec::new())
    }
}
