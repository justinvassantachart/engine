use anyhow::Result;
use gimli::{DwarfSections, EndianRcSlice, LittleEndian, SectionId};
use std::collections::HashMap;
use std::rc::Rc;
use wasmparser::{Parser, Payload};

/// The reader type we use any time we interface with `gimli`.
type Reader = EndianRcSlice<LittleEndian>;

#[derive(Clone)]
pub struct Dwarf {
    /// Provides direct access to `gimli`
    inner: gimli::Dwarf<Reader>,
    /// DWARF sections maintained for serialization.
    /// References same memory as `inner`.
    sections: gimli::DwarfSections<Reader>,
}

impl Dwarf {
    /// Load Dwarf from section map
    pub fn from_sections(sections: &HashMap<&str, &[u8]>) -> Result<Self> {
        let sections = DwarfSections::load(|id: SectionId| -> Result<Reader, gimli::Error> {
            let data = sections.get(id.name()).copied().unwrap_or(&[]);
            Ok(EndianRcSlice::new(Rc::from(data), LittleEndian))
        })?;
        let inner = sections.borrow(|section| section.clone());
        Ok(Self { inner, sections })
    }
}

pub mod serde {
    use std::collections::HashMap;

    use super::{Dwarf, Reader};
    use gimli::Section;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    fn insert<S: Section<Reader>>(m: &mut HashMap<String, Vec<u8>>, section: &S) {
        let bytes = section.reader().bytes();
        if bytes.len() > 0 {
            m.insert(S::id().name().to_string(), bytes.to_vec());
        }
    }

    pub fn serialize<S>(dwarf: &Dwarf, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let sec = &dwarf.sections;
        let mut m = HashMap::new();
        insert(&mut m, &sec.debug_abbrev);
        insert(&mut m, &sec.debug_addr);
        insert(&mut m, &sec.debug_aranges);
        insert(&mut m, &sec.debug_info);
        insert(&mut m, &sec.debug_line);
        insert(&mut m, &sec.debug_line_str);
        insert(&mut m, &sec.debug_macinfo);
        insert(&mut m, &sec.debug_macro);
        insert(&mut m, &sec.debug_names);
        insert(&mut m, &sec.debug_str);
        insert(&mut m, &sec.debug_str_offsets);
        insert(&mut m, &sec.debug_types);
        insert(&mut m, &sec.debug_loc);
        insert(&mut m, &sec.debug_loclists);
        insert(&mut m, &sec.debug_ranges);
        insert(&mut m, &sec.debug_rnglists);
        m.serialize(s)
    }

    pub fn deserialize<'de, D>(d: D) -> Result<Dwarf, D::Error>
    where
        D: Deserializer<'de>,
    {
        let map: HashMap<String, Vec<u8>> = HashMap::deserialize(d)?;
        let map: HashMap<&str, &[u8]> = map
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
            .collect();
        super::Dwarf::from_sections(&map).map_err(serde::de::Error::custom)
    }
}
