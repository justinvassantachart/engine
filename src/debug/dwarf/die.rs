use std::collections::VecDeque;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::util::weak_error;

use super::{Dwarf, R, Unit};
use gimli::Reader;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DieReference {
    unit_index: usize,
    #[serde(with = "crate::debug::dwarf::serde::unit_offset")]
    unit_ofs: gimli::UnitOffset,
}

impl DieReference {
    pub fn deref<'a>(&self, dwarf: &'a Dwarf) -> Result<Die<'a>> {
        let Some(unit) = dwarf.units.get(self.unit_index) else {
            return Err(anyhow!("Unit index out of bounds"));
        };

        let die = unit.unit().entry(self.unit_ofs)?;

        Ok(Die::new(DerefContext::new(dwarf, unit), die))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DerefContext<'a> {
    pub dwarf: &'a Dwarf,
    pub unit: &'a Unit,
}

impl<'a> DerefContext<'a> {
    #[inline]
    pub fn new(dwarf: &'a Dwarf, unit: &'a Unit) -> Self {
        Self { dwarf, unit }
    }

    #[inline]
    pub(crate) fn unit_ref(self) -> gimli::UnitRef<'a, R> {
        gimli::UnitRef::new(&self.dwarf.inner, self.unit.unit())
    }
}

pub struct Die<'a> {
    ctx: DerefContext<'a>,
    die: gimli::DebuggingInformationEntry<R>,
}

impl<'a> Die<'a> {
    pub(crate) fn new(ctx: DerefContext<'a>, die: gimli::DebuggingInformationEntry<R>) -> Self {
        Self { ctx, die }
    }

    pub fn name(&self) -> Option<String> {
        self.attr_to_string(gimli::DW_AT_name)
    }

    pub fn die(&self) -> &gimli::DebuggingInformationEntry<R> {
        &self.die
    }

    pub fn die_ref(&self) -> DieReference {
        DieReference {
            unit_index: self.ctx.unit.index(),
            unit_ofs: self.die.offset,
        }
    }

    pub fn attr_to_string(&self, attr: gimli::DwAt) -> Option<String> {
        weak_error!(
            self.die
                .attr(attr)
                .and_then(|attr| self.ctx.unit_ref().attr_string(attr.value()).ok())
                .map(|l| l.to_string_lossy().map(|s| s.to_string()))
                .transpose()
        )
        .unwrap_or(None)
    }

    /// Loop through child DIEs until a value is found, returning it (if any)
    pub fn find_children<T>(&self, mut f: impl FnMut(Die<'a>) -> Option<T>) -> Option<T> {
        let mut tree = weak_error!(self.ctx.unit_ref().entries_tree(Some(self.die.offset())))?;

        let root = weak_error!(tree.root())?;
        let mut children = root.children();
        while let Some(c) = weak_error!(children.next())? {
            let die = Die::new(self.ctx, c.entry().clone());

            if let Some(r) = f(die) {
                return Some(r);
            }
        }

        None
    }

    /// Loop through all child DIEs, performing an action on each
    pub fn for_each_child(&self, mut f: impl FnMut(Die)) {
        self.find_children::<()>(|die| {
            f(die);
            None
        });
    }

    /// Loop through all children DIEs, collecting a value for each (if any)
    pub fn collect_children<T>(&self, mut f: impl FnMut(Die) -> Option<T>) -> Vec<T> {
        let mut result = vec![];
        self.for_each_child(|die| {
            if let Some(r) = f(die) {
                result.push(r);
            }
        });

        result
    }

    pub fn traverse(&self, mut f: impl FnMut(Die) -> Visit) {
        let mut queue = VecDeque::from([self.die.offset()]);

        while let Some(offset) = queue.pop_front() {
            let Some(mut tree) = weak_error!(self.ctx.unit.unit().entries_tree(Some(offset)))
            else {
                return;
            };

            let Some(root) = weak_error!(tree.root()) else {
                return;
            };

            let mut children = root.children();
            while let Some(Some(child)) = weak_error!(children.next()) {
                let die = Die::new(self.ctx.clone(), child.entry().clone());
                match f(die) {
                    Visit::Continue => queue.push_back(offset),
                    Visit::SkipChildren => {}
                    Visit::Break => return,
                }
            }
        }
    }
}

pub enum Visit {
    /// Traverse the next node, including children
    Continue,
    /// Skip the children of this node
    SkipChildren,
    /// Stop traversal immediately
    Break,
}
