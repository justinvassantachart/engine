use crate::debug::BREAKPOINT_PREFIX_BYTES;
use crate::types::{DebugFunction, DebugInfo, LocationInfo, MemoryDescriptor};
use gimli::read::ReaderOffset;
use gimli::{EndianSlice, LittleEndian, Reader, SectionId};
use std::borrow::Cow;
use std::collections::HashMap;
use wasmparser::{Parser, Payload};

/// Parse DWARF debug info from WASM bytes.
pub fn parse_debug_info(wasm_bytes: &[u8]) -> anyhow::Result<DebugInfo> {
    let mut sections: HashMap<&str, &[u8]> = HashMap::new();
    let mut memory_initial = 0u32;

    for payload in Parser::new(0).parse_all(wasm_bytes) {
        let payload = payload?;
        match payload {
            Payload::CustomSection(reader) => {
                sections.insert(reader.name(), reader.data());
            }
            Payload::MemorySection(reader) => {
                for mem in reader {
                    let mem = mem?;
                    memory_initial = mem.initial as u32;
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

    let mut locations = Vec::new();
    let mut files = Vec::new();
    let mut functions = Vec::new();
    let mut file_map: HashMap<String, usize> = HashMap::new();

    let mut units = dwarf.units();
    while let Some(header) = units.next()? {
        let unit = dwarf.unit(header)?;
        parse_unit_functions(&dwarf, &unit, &mut functions)?;
        parse_unit_lines(&dwarf, &unit, &mut locations, &mut files, &mut file_map)?;
    }

    let breakpoints = create_breakpoints_buffer(locations.len());
    let memory = MemoryDescriptor::new(memory_initial, 16 * memory_initial);
    let stack = MemoryDescriptor::new(16, 16);
    let dwarf = collect_dwarf_bytes(&sections);

    Ok(DebugInfo {
        locations,
        files,
        functions,
        breakpoints,
        memory,
        stack,
        dwarf,
    })
}

fn create_breakpoints_buffer(num_locations: usize) -> js_sys::SharedArrayBuffer {
    // 3 u32s for metadata and rest is breakpoint status
    js_sys::SharedArrayBuffer::new((BREAKPOINT_PREFIX_BYTES + num_locations) as u32)
}

fn collect_dwarf_bytes(sections: &HashMap<&str, &[u8]>) -> HashMap<String, Vec<u8>> {
    let mut out = HashMap::new();
    for name in [
        ".debug_abbrev",
        ".debug_info",
        ".debug_line",
        ".debug_line_str",
        ".debug_str",
    ] {
        if let Some(data) = sections.get(name) {
            out.insert(name.to_string(), data.to_vec());
        }
    }
    out
}

pub fn to_dwarf<'a>(
    bytes: &'a HashMap<String, Vec<u8>>,
) -> gimli::Dwarf<EndianSlice<'a, LittleEndian>> {
    let load = |id: SectionId| -> Result<EndianSlice<'a, LittleEndian>, gimli::Error> {
        let name = id.name();
        let data = bytes.get(name).map(|v| &v[..]).unwrap_or(&[]);
        Ok(EndianSlice::new(data, LittleEndian))
    };

    gimli::Dwarf::load(&load).unwrap()
}

// ============================================================================
// .debug_info: functions (address and DIE offset only)
// ============================================================================

fn parse_unit_functions<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    functions: &mut Vec<DebugFunction>,
) -> Result<(), gimli::Error> {
    let mut tree = unit.entries_tree(None)?;
    let root = tree.root()?;
    let mut children = root.children();

    while let Some(child) = children.next()? {
        collect_subprograms(dwarf, unit, child, functions)?;
    }

    Ok(())
}

fn collect_subprograms<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    node: gimli::EntriesTreeNode<'_, '_, R>,
    functions: &mut Vec<DebugFunction>,
) -> Result<(), gimli::Error> {
    let tag = node.entry().tag();

    if tag == gimli::DW_TAG_subprogram {
        if let Some((low_pc, _high_pc)) = get_pc_range(node.entry()) {
            let name = node
                .entry()
                .attr(gimli::DW_AT_name)
                .and_then(|a| dwarf.attr_string(unit, a.value()).ok())
                .and_then(|s| s.to_string_lossy().ok().map(|cow| cow.into_owned()))
                .unwrap_or_else(|| format!("function_{}", functions.len()));
            functions.push(DebugFunction {
                unit: unit.offset().0.into_u64() as usize,
                offset: node.entry().offset().0.into_u64() as usize,
                address: low_pc as usize,
                size: 4,
                layout: vec![],
                name,
            });
        }
        return Ok(());
    }

    if matches!(
        tag,
        gimli::DW_TAG_namespace
            | gimli::DW_TAG_module
            | gimli::DW_TAG_class_type
            | gimli::DW_TAG_structure_type
            | gimli::DW_TAG_union_type
    ) {
        let mut children = node.children();
        while let Some(child) = children.next()? {
            collect_subprograms(dwarf, unit, child, functions)?;
        }
    }

    Ok(())
}

fn get_pc_range<R: Reader>(entry: &gimli::DebuggingInformationEntry<R>) -> Option<(u64, u64)> {
    let low_pc = match entry.attr(gimli::DW_AT_low_pc)?.value() {
        gimli::AttributeValue::Addr(a) => a,
        _ => return None,
    };
    let high_pc = match entry.attr(gimli::DW_AT_high_pc)?.value() {
        gimli::AttributeValue::Addr(a) => a,
        gimli::AttributeValue::Udata(offset) => low_pc + offset,
        _ => return None,
    };
    Some((low_pc, high_pc))
}

// ============================================================================
// .debug_line: breakpoint locations
// ============================================================================

fn parse_unit_lines<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    locations: &mut Vec<LocationInfo>,
    files: &mut Vec<String>,
    file_map: &mut HashMap<String, usize>,
) -> Result<(), gimli::Error> {
    let Some(program) = unit.line_program.clone() else {
        return Ok(());
    };

    let mut rows = program.rows();
    while let Some((header, row)) = rows.next_row()? {
        if !row.is_stmt() {
            continue;
        }

        let Some(file_entry) = row.file(header) else {
            continue;
        };

        let filename = build_filename(dwarf, unit, file_entry)?;

        let file_idx = if let Some(&idx) = file_map.get(&filename) {
            idx
        } else {
            let idx = files.len();
            files.push(filename.clone());
            file_map.insert(filename, idx);
            idx
        };

        let line = row.line().map(|l| l.get()).unwrap_or(0) as usize;
        let col = match row.column() {
            gimli::ColumnType::LeftEdge => 0,
            gimli::ColumnType::Column(c) => c.get() as usize,
        };

        locations.push(LocationInfo {
            file: file_idx,
            line,
            col,
            address: row.address() as usize,
        });
    }

    Ok(())
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
