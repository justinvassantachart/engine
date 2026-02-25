use crate::types::{DebugInfo, LocationInfo};
use gimli::{EndianSlice, LittleEndian, Reader};
use object::{Object, ObjectSection};
use std::borrow::Cow;
use std::collections::HashMap;

/// Parse DWARF debug info from WASM bytes
pub fn parse_debug_info(wasm_bytes: &[u8]) -> Result<DebugInfo, String> {
    let object =
        object::File::parse(wasm_bytes).map_err(|e| format!("Failed to parse WASM: {:?}", e))?;

    let load_section = |id: gimli::SectionId| -> Result<Cow<'_, [u8]>, gimli::Error> {
        Ok(object
            .section_by_name(id.name())
            .and_then(|s| s.uncompressed_data().ok())
            .unwrap_or(Cow::Borrowed(&[])))
    };

    let dwarf_sections = gimli::DwarfSections::load(load_section)
        .map_err(|e| format!("Failed to load DWARF sections: {:?}", e))?;
    let dwarf =
        dwarf_sections.borrow(|section| EndianSlice::new(Cow::as_ref(section), LittleEndian));

    let mut locations = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut file_map: HashMap<String, u32> = HashMap::new();

    let mut units = dwarf.units();
    while let Some(header) = units.next().map_err(|e| format!("{:?}", e))? {
        let unit = dwarf.unit(header).map_err(|e| format!("{:?}", e))?;

        let Some(program) = unit.line_program.clone() else {
            continue;
        };

        let mut rows = program.rows();
        while let Some((header, row)) = rows.next_row().map_err(|e| format!("{:?}", e))? {
            if !row.is_stmt() {
                continue;
            }

            let Some(file_entry) = row.file(header) else {
                continue;
            };

            let filename =
                build_filename(&dwarf, &unit, file_entry).map_err(|e| format!("{:?}", e))?;

            let file_idx = if let Some(&idx) = file_map.get(&filename) {
                idx
            } else {
                let idx = files.len() as u32;
                files.push(filename.clone());
                file_map.insert(filename, idx);
                idx
            };

            let line = row.line().map(|l| l.get()).unwrap_or(0) as u32;
            let col = match row.column() {
                gimli::ColumnType::LeftEdge => 0,
                gimli::ColumnType::Column(c) => c.get() as u32,
            };

            locations.push(LocationInfo {
                file: file_idx,
                line,
                col,
                address: row.address(),
            });
        }
    }

    Ok(DebugInfo { locations, files })
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
