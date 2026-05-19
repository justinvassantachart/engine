//! `std::map` and `std::set` formatters (`std::__tree`).
//!
//! See the corresponding [LLDB formatter code](https://github.com/llvm/llvm-project/blob/main/lldb/source/Plugins/Language/CPlusPlus/LibCxxMap.cpp)
//! for reference.

use std::ops::Range;

use anyhow::{Context, Result};

use crate::debug::Debugger;
use crate::debug::Variable;
use crate::debug::formatters::{ChildCounts, VariableFormatter};
use crate::types::GlobalAddress;
use crate::util::WeakRef;

const TREE_NODE_VALUE_OFFSET: u64 = 16;

/// True for `std::map<...>` / `std::set<...>` container instantiations, not nested names
/// like `std::map<...>::value_type` (which also start with `std::map<`).
fn is_container_instantiation(name: &str, container: &str) -> bool {
    name.starts_with(&format!("std::{container}<")) && !name.contains(">::")
}

#[derive(Clone)]
struct TreeEntry {
    addr: u64,
    debugger: WeakRef<Debugger>,
}

impl TreeEntry {
    fn new(addr: u64, debugger: WeakRef<Debugger>) -> Self {
        Self { addr, debugger }
    }

    fn null(&self) -> bool {
        self.addr == 0
    }

    fn ptr_size(&self) -> u64 {
        self.debugger
            .as_deref()
            .map(|dbg| dbg.pointer_size())
            .unwrap_or(4)
    }

    fn read_ptr(&self, offset: u64) -> u64 {
        self.debugger
            .as_deref()
            .map(|dbg| {
                dbg.memory()
                    .read_pointer(GlobalAddress(self.addr + offset))
                    .0
            })
            .unwrap_or(0)
    }

    fn left(&self) -> Self {
        Self::new(self.read_ptr(0), self.debugger.clone())
    }

    fn right(&self) -> Self {
        let ptr_size = self.ptr_size();
        Self::new(self.read_ptr(ptr_size), self.debugger.clone())
    }

    fn parent(&self) -> Self {
        let ptr_size = self.ptr_size();
        Self::new(self.read_ptr(2 * ptr_size), self.debugger.clone())
    }
}

struct TreeIter {
    entry: TreeEntry,
    max_depth: usize,
    error: bool,
}

impl TreeIter {
    fn new(entry: TreeEntry, max_depth: usize) -> Self {
        Self {
            entry,
            max_depth,
            error: false,
        }
    }

    fn advance(&mut self, count: usize) -> Option<u64> {
        if self.error {
            return None;
        }
        let mut steps = 0;
        for _ in 0..count {
            self.next();
            steps += 1;
            if self.error || self.entry.null() || steps > self.max_depth {
                return None;
            }
        }
        Some(self.entry.addr)
    }

    fn next(&mut self) {
        if self.entry.null() {
            return;
        }
        let right = self.entry.right();
        if !right.null() {
            self.entry = self.tree_min(right);
            return;
        }
        let mut steps = 0;
        while !self.is_left_child(&self.entry) {
            self.entry = self.entry.parent();
            steps += 1;
            if steps > self.max_depth {
                self.entry = TreeEntry::new(0, self.entry.debugger.clone());
                return;
            }
        }
        self.entry = self.entry.parent();
    }

    fn tree_min(&mut self, mut x: TreeEntry) -> TreeEntry {
        if x.null() {
            return TreeEntry::new(0, x.debugger.clone());
        }
        let mut steps = 0;
        loop {
            let left = x.left();
            if left.null() {
                return x;
            }
            x = left;
            steps += 1;
            if steps > self.max_depth {
                self.error = true;
                return TreeEntry::new(0, x.debugger.clone());
            }
        }
    }

    fn is_left_child(&self, x: &TreeEntry) -> bool {
        if x.null() {
            return false;
        }
        let parent = x.parent();
        if parent.null() {
            return false;
        }
        parent.left().addr == x.addr
    }
}

struct LibcxxTree<'a> {
    value: &'a Variable,
    begin: u64,
    count: usize,
    value_ty: crate::debug::Type,
    debugger: WeakRef<Debugger>,
}

impl<'a> LibcxxTree<'a> {
    fn new(value: &'a Variable) -> Result<Self> {
        let debugger = value.debugger_reference();
        let tree = value.named_child("__tree_")?;
        let begin = tree
            .named_child("__begin_node_")?
            .pointer_value()
            .context("std::__tree __begin_node_ is unavailable")?
            .0;
        let count = tree_size(&tree)? as usize;
        let value_ty = node_value_type(&tree)?;
        Ok(Self {
            value,
            begin,
            count,
            value_ty,
            debugger,
        })
    }

    fn count(&self) -> usize {
        self.count
    }

    fn indexed_children(&self, range: Range<usize>) -> Result<Vec<Variable>> {
        let start = range.start.min(self.count);
        let end = range.end.min(self.count);
        let mut out = Vec::with_capacity(end.saturating_sub(start));
        let mut iter = TreeIter::new(
            TreeEntry::new(self.begin, self.debugger.clone()),
            self.count,
        );
        if iter.advance(start).is_none() {
            return Ok(out);
        }
        for i in start..end {
            out.push(self.value.copy(
                format!("[{i}]"),
                GlobalAddress(iter.entry.addr + TREE_NODE_VALUE_OFFSET).pieces(),
                self.value_ty.clone(),
            ));
            if i + 1 < end {
                iter.next();
                if iter.error || iter.entry.null() {
                    break;
                }
            }
        }
        Ok(out)
    }
}

fn tree_size(tree: &Variable) -> Result<u64> {
    tree.named_child("__size_")?
        .u64_value()
        .context("Could not read u64 value")
}

fn node_value_type(tree: &Variable) -> Result<crate::debug::Type> {
    Ok(tree
        .ty()
        .discard_modifiers()
        .direct_nested_type_with_name("value_type")?
        .discard_modifiers())
}

fn tree_indexed_children(value: &Variable, range: Range<usize>) -> Result<Vec<Variable>> {
    LibcxxTree::new(value)?.indexed_children(range)
}

pub struct StdMapFormatter;

impl VariableFormatter for StdMapFormatter {
    fn matches(&self, value: &Variable) -> bool {
        let name = value.ty().name();
        value.ty().ns().matches("std")
            && (name == "std::map" || is_container_instantiation(&name, "map"))
    }

    fn display(&self, value: &Variable) -> Result<String> {
        value.display()
    }

    fn num_children(&self, value: &Variable) -> Result<ChildCounts> {
        let tree = LibcxxTree::new(value)?;
        Ok(ChildCounts::indexed(tree.count()))
    }

    fn indexed_children(&self, value: &Variable, range: Range<usize>) -> Result<Vec<Variable>> {
        tree_indexed_children(value, range)
    }

    fn named_children(&self, _value: &Variable, _range: Range<usize>) -> Result<Vec<Variable>> {
        Ok(Vec::new())
    }
}

pub struct StdSetFormatter;

impl VariableFormatter for StdSetFormatter {
    fn matches(&self, value: &Variable) -> bool {
        let name = value.ty().name();
        value.ty().ns().matches("std")
            && (name == "std::set" || is_container_instantiation(&name, "set"))
    }

    fn display(&self, value: &Variable) -> Result<String> {
        value.display()
    }

    fn num_children(&self, value: &Variable) -> Result<ChildCounts> {
        let tree = LibcxxTree::new(value)?;
        Ok(ChildCounts::indexed(tree.count()))
    }

    fn indexed_children(&self, value: &Variable, range: Range<usize>) -> Result<Vec<Variable>> {
        tree_indexed_children(value, range)
    }

    fn named_children(&self, _value: &Variable, _range: Range<usize>) -> Result<Vec<Variable>> {
        Ok(Vec::new())
    }
}
