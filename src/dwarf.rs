use crate::types::{DebugInfo, LocationInfo};
use gimli::{EndianSlice, LittleEndian, Reader};
use std::borrow::Cow;
use std::collections::HashMap;
use wasmparser::{Parser, Payload};

/// Parse DWARF debug info from WASM bytes
pub fn parse_debug_info(wasm_bytes: &[u8]) -> anyhow::Result<DebugInfo> {
    let mut info = DebugInfo::default();
    let mut sections: HashMap<&str, &[u8]> = HashMap::new();

    for payload in Parser::new(0).parse_all(wasm_bytes) {
        let payload = payload?;
        match payload {
            Payload::CustomSection(reader) => {
                sections.insert(reader.name(), reader.data());
            }
            Payload::MemorySection(reader) => {
                for mem in reader {
                    info.memory.initial_pages = mem?.initial;
                    info.memory.maximum_pages = info.memory.initial_pages * 16;
                    break;
                }
            }
            _ => {}
        }
    }

    let load_section = |id: gimli::SectionId| -> Result<Cow<'_, [u8]>, gimli::Error> {
        Ok(sections
            .get(id.name())
            .map(|data| Cow::Borrowed(*data))
            .unwrap_or(Cow::Borrowed(&[])))
    };

    let dwarf_sections = gimli::DwarfSections::load(load_section)?;
    let dwarf =
        dwarf_sections.borrow(|section| EndianSlice::new(Cow::as_ref(section), LittleEndian));

    let mut file_map: HashMap<String, u32> = HashMap::new();

    let mut units = dwarf.units();
    while let Some(header) = units.next()? {
        let unit = dwarf.unit(header)?;

        let Some(program) = unit.line_program.clone() else {
            continue;
        };

        let mut rows = program.rows();
        while let Some((header, row)) = rows.next_row()? {
            if !row.is_stmt() {
                continue;
            }

            let Some(file_entry) = row.file(header) else {
                continue;
            };

            let filename = build_filename(&dwarf, &unit, file_entry)?;

            let file_idx = if let Some(&idx) = file_map.get(&filename) {
                idx
            } else {
                let idx = info.files.len() as u32;
                info.files.push(filename.clone());
                file_map.insert(filename, idx);
                idx
            };

            let line = row.line().map(|l| l.get()).unwrap_or(0) as u32;
            let col = match row.column() {
                gimli::ColumnType::LeftEdge => 0,
                gimli::ColumnType::Column(c) => c.get() as u32,
            };

            info.locations.push(LocationInfo {
                file: file_idx,
                line,
                col,
                address: row.address(),
            });
        }
    }

    Ok(info)
}

fn build_filename<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    file_entry: &gimli::FileEntry<R>,
) -> Result<String, gimli::Error> {
    let mut path = String::new();

    if let Some(dir) = file_entry.directory(unit.line_program.as_ref().unwrap().header()) {
        let dir_str = dwarf.attr_string(unit, dir)?;
        let dir_str = dir_str.to_string_lossy()?;
        if !dir_str.is_empty() && dir_str != "." {
            path.push_str(&dir_str);
            if !path.ends_with('/') {
                path.push('/');
            }
        }
    }

    let name = dwarf.attr_string(unit, file_entry.path_name())?;
    path.push_str(&name.to_string_lossy()?);

    Ok(path)
}
