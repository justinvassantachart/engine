use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::{Dwarf, R};
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

        let unit = unit.unit();
        let die = unit.entry(self.unit_ofs)?;

        Ok(Die {
            dwarf: &dwarf.inner,
            unit,
            die,
        })
    }
}

pub struct Die<'a> {
    dwarf: &'a gimli::Dwarf<R>,
    unit: &'a gimli::Unit<R>,
    die: gimli::DebuggingInformationEntry<R>,
}

impl<'a> Die<'a> {
    pub fn name(&self) -> Option<String> {
        self.attr_to_string(gimli::DW_AT_name)
    }

    pub fn attr_to_string(&self, attr: gimli::DwAt) -> Option<String> {
        self.die
            .attr(attr)
            .and_then(|attr| self.dwarf.attr_string(self.unit, attr.value()).ok())
            .map(|l| l.to_string_lossy().map(|s| s.to_string()))
            .transpose()
            .ok()
            .unwrap_or(None)
    }
}
