use crate::types::LocationInfo;
use gimli::{EndianSlice, LittleEndian, Reader};
use object::{Object, ObjectSection};
use std::borrow::Cow;
use std::collections::HashMap;

type GimliReader<'a> = EndianSlice<'a, LittleEndian>;

/// Parse DWARF debug info from WASM bytes to extract breakpoint locations.
///
/// Returns (locations, files) where:
/// - locations: All possible breakpoint locations (file index, line, col)
/// - files: Deduplicated list of source filenames
pub fn parse_dwarf_info(wasm_bytes: &[u8]) -> (Vec<LocationInfo>, Vec<String>) {
    match parse_dwarf_inner(wasm_bytes) {
        Ok(result) => result,
        Err(e) => {
            web_sys::console::error_1(&format!("DWARF parsing error: {:?}", e).into());
            (vec![], vec![])
        }
    }
}

fn parse_dwarf_inner(wasm_bytes: &[u8]) -> Result<(Vec<LocationInfo>, Vec<String>), gimli::Error> {
    // Parse the WASM file
    let object = match object::File::parse(wasm_bytes) {
        Ok(obj) => obj,
        Err(e) => {
            web_sys::console::error_1(&format!("Failed to parse WASM: {:?}", e).into());
            return Ok((vec![], vec![]));
        }
    };

    // Load DWARF sections from the WASM file
    let load_section = |id: gimli::SectionId| -> Result<Cow<'_, [u8]>, gimli::Error> {
        Ok(object
            .section_by_name(id.name())
            .and_then(|s| s.uncompressed_data().ok())
            .unwrap_or(Cow::Borrowed(&[])))
    };

    let dwarf_sections = gimli::DwarfSections::load(load_section)?;
    let dwarf = dwarf_sections.borrow(|section| {
        EndianSlice::new(Cow::as_ref(section), LittleEndian)
    });

    let mut locations = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut file_map: HashMap<String, u32> = HashMap::new();

    // Iterate over compilation units
    let mut units = dwarf.units();
    while let Some(header) = units.next()? {
        let unit = dwarf.unit(header)?;

        // Get the line program for this unit
        let Some(program) = unit.line_program.clone() else {
            continue;
        };

        // Execute the line program to get all rows
        let mut rows = program.rows();
        while let Some((header, row)) = rows.next_row()? {
            // Skip rows that aren't statement beginnings (not useful for breakpoints)
            if !row.is_stmt() {
                continue;
            }

            // Get the file entry
            let Some(file_entry) = row.file(header) else {
                continue;
            };

            // Build the filename
            let filename = build_filename(&dwarf, &unit, file_entry)?;

            // Get or insert file index
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
            });
        }
    }

    Ok((locations, files))
}

/// Build a filename from a file entry, handling directory prefixes.
fn build_filename<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    file_entry: &gimli::FileEntry<R>,
) -> Result<String, gimli::Error> {
    let mut path = String::new();

    // Add directory if present
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

    // Add filename
    let name = dwarf.attr_string(unit, file_entry.path_name())?;
    path.push_str(&name.to_string_lossy()?);

    Ok(path)
}
