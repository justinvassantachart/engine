//! `std::map` and `std::set` formatters (`std::__tree`).
//!
//! See the corresponding [LLDB formatter code](https://github.com/llvm/llvm-project/blob/main/lldb/source/Plugins/Language/CPlusPlus/LibCxxMap.cpp)
//! for reference.

use std::ops::Range;

use anyhow::{Context, Result};

use crate::debug::Type;
use crate::debug::Variable;
use crate::debug::formatters::{ChildCounts, VariableFormatter};
use crate::types::GlobalAddress;

const VALUE_OFFSET: u64 = 16;

struct LibcxxTree<'a> {
    value: &'a Variable,
    node: Variable,
    pos: u64,
    count: usize,
    value_ty: Type,
    ptr_size: u64,
}

impl<'a> LibcxxTree<'a> {
    fn new(value: &'a Variable) -> Result<Self> {
        let tree = value
            .named_child("__tree_")
            .context("No child named '__tree_'")?;
        let node = tree
            .named_child("__begin_node_")
            .context("No child named '__begin_node_'")?;
        let pos = node.pointer_value().map(|a| a.0).unwrap_or(0);
        let ptr_size = node.debugger().map(|d| d.pointer_size()).unwrap_or(4);
        let value_ty = tree
            .ty()
            .discard_modifiers()
            .direct_nested_type_with_name("value_type")?
            .discard_modifiers();
        Ok(Self {
            value,
            node,
            pos,
            count: tree
                .named_child("__size_")
                .context("No child named '__size_'")?
                .u64_value()
                .context("Could not read __size_")? as usize,
            value_ty,
            ptr_size,
        })
    }

    fn link(&self, node: u64, offset: u64) -> u64 {
        if node == 0 {
            return 0;
        }
        self.node
            .debugger()
            .map(|dbg| dbg.memory().read_pointer(GlobalAddress(node + offset)).0)
            .unwrap_or(0)
    }

    fn indexed_children(&self, range: Range<usize>) -> Result<Vec<Variable>> {
        let end = range.end.min(self.count);
        let mut iter = TreeIter::new(self);
        let Some(mut pos) = iter.nth(range.start) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::with_capacity(end - range.start);
        for i in range.start..end {
            out.push(self.value.copy(
                format!("[{i}]"),
                GlobalAddress(pos + VALUE_OFFSET).pieces(),
                self.value_ty.clone(),
            ));
            if i + 1 < end {
                let Some(next) = iter.next() else {
                    break;
                };
                pos = next;
            }
        }
        Ok(out)
    }
}

struct TreeIter<'a> {
    tree: &'a LibcxxTree<'a>,
    pos: u64,
    steps: usize,
    done: bool,
}

impl<'a> TreeIter<'a> {
    fn new(tree: &'a LibcxxTree<'a>) -> Self {
        Self {
            tree,
            pos: tree.pos,
            steps: 0,
            done: false,
        }
    }

    fn nth(&mut self, n: usize) -> Option<u64> {
        for _ in 0..n {
            self.next()?;
        }
        (self.pos != 0).then_some(self.pos)
    }

    fn next(&mut self) -> Option<u64> {
        if self.done || self.pos == 0 {
            return None;
        }
        let t = self.tree;
        let ptr_size = t.ptr_size;
        let right = t.link(self.pos, ptr_size);
        if right != 0 {
            self.pos = self.min(right);
            return (self.pos != 0).then_some(self.pos);
        }
        while !self.is_left_child(self.pos) {
            self.pos = t.link(self.pos, 2 * ptr_size);
            self.steps += 1;
            if self.pos == 0 || self.steps > t.count {
                self.done = true;
                return None;
            }
        }
        self.pos = t.link(self.pos, 2 * ptr_size);
        (self.pos != 0).then_some(self.pos)
    }

    fn min(&mut self, mut x: u64) -> u64 {
        loop {
            let left = self.tree.link(x, 0);
            if left == 0 {
                return x;
            }
            x = left;
            self.steps += 1;
            if self.steps > self.tree.count {
                self.done = true;
                return 0;
            }
        }
    }

    fn is_left_child(&self, node: u64) -> bool {
        let parent = self.tree.link(node, 2 * self.tree.ptr_size);
        parent != 0 && self.tree.link(parent, 0) == node
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
