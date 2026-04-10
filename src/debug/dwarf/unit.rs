use super::R;
use anyhow::Result;

#[derive(Debug)]
pub struct Unit {
    /// Provides direct access to `gimli`
    unit: gimli::Unit<R>,
}

impl Unit {
    pub fn clone(&self, dwarf: &gimli::Dwarf<R>) -> Self {
        let unit = {
            dwarf
                .unit(self.unit.header.clone())
                .expect("clone unit should not fail")
        };

        Self { unit }
    }

    pub fn new(unit: gimli::Unit<R>) -> Result<Unit> {
        Ok(Unit { unit })
    }

    pub fn unit(&self) -> &gimli::Unit<R> {
        &self.unit
    }
}
