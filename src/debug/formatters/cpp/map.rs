//! `std::map` and `std::set` formatters (`std::__tree`).

use std::ops::Range;

use anyhow::{Context, Result};

use crate::debug::Debugger;
use crate::debug::Variable;
use crate::debug::formatters::{ChildCounts, VariableFormatter};
use crate::types::GlobalAddress;
use crate::util::WeakRef;

const PTR_SIZE: u64 = 4;
const TREE_NODE_VALUE_OFFSET: u64 = 16;

#[derive(Clone)]
struct TreeEntry {
    addr: u64,
    debugger: WeakRef<Debugger>,
}

impl TreeEntry {
    fn new(addr: u64, debugger: WeakRef<Debugger>) -> Self {
        Self { addr, debugger }
    }

    fn null(self) -> bool {
        self.addr == 0
    }

    fn read_ptr(self, offset: u64) -> u64 {
        self.debugger
            .as_deref()
            .map(|dbg| {
                dbg.memory()
                    .read_pointer(GlobalAddress(self.addr + offset))
                    .0
            })
            .unwrap_or(0)
    }

    fn left(self) -> Self {
        Self::new(self.read_ptr(0), self.debugger)
    }

    fn right(self) -> Self {
        Self::new(self.read_ptr(PTR_SIZE), self.debugger)
    }

    fn parent(self) -> Self {
        Self::new(self.read_ptr(2 * PTR_SIZE), self.debugger)
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
        while !self.is_left_child(self.entry) {
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
            return TreeEntry::new(0, x.debugger);
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
                return TreeEntry::new(0, x.debugger);
            }
        }
    }

    fn is_left_child(&self, x: TreeEntry) -> bool {
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
        let count = tree_size(&tree)?;
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

fn tree_size(tree: &Variable) -> Result<usize> {
    let size = tree.named_child("__size_")?;
    let bytes = size
        .read(PTR_SIZE as usize)
        .context("std::__tree __size_ is unavailable")?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap_or([0; 4])) as usize)
}

fn node_value_type(tree: &Variable) -> Result<crate::debug::Type> {
    tree.ty()
        .discard_modifiers()
        .context("std::__tree type is unavailable")?
        .direct_nested_type_with_name("__node_pointer")
        .context("std::__tree is missing __node_pointer")?
        .pointee()
        .context("std::__tree __node_pointer has no pointee")?
        .member("__value_")
        .context("std::__tree_node is missing __value_")
}

fn tree_indexed_children(value: &Variable, range: Range<usize>) -> Result<Vec<Variable>> {
    LibcxxTree::new(value)?.indexed_children(range)
}

pub struct StdMapFormatter;

impl VariableFormatter for StdMapFormatter {
    fn matches(&self, value: &Variable) -> bool {
        let name = value.ty().name();
        value.ty().ns().matches("std") && (name == "std::map" || name.starts_with("std::map<"))
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
        value.ty().ns().matches("std") && (name == "std::set" || name.starts_with("std::set<"))
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
