use anyhow::Result;
use gimli::{DwarfSections, EndianRcSlice, LittleEndian, SectionId};
use std::collections::HashMap;
use std::rc::Rc;
use wasmparser::{Parser, Payload};

pub struct Dwarf {
    inner: gimli::Dwarf<EndianRcSlice<LittleEndian>>,
}

impl Dwarf {
    /// Load Dwarf from a WASM binary.
    pub fn from_wasm(wasm: &[u8]) -> Result<Self> {
        let mut sections: HashMap<&str, &[u8]> = HashMap::new();
        for payload in Parser::new(0).parse_all(wasm) {
            let payload = payload?;
            if let Payload::CustomSection(reader) = payload {
                sections.insert(reader.name(), reader.data());
            }
        }

        let dwarf_sections =
            DwarfSections::load(|id: SectionId| -> Result<Rc<[u8]>, gimli::Error> {
                let data = sections.get(id.name()).copied().unwrap_or(&[]);
                Ok(Rc::from(data))
            })?;

        let inner =
            dwarf_sections.borrow(|section| EndianRcSlice::new(section.clone(), LittleEndian));

        Ok(Self { inner })
    }
}
