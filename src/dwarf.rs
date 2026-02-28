use crate::types::{
    DebugFunction, DebugInfo, DebugVariable, DwarfOp, LocationInfo, VarLocationRange,
};
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
                    let mem = mem?;
                    info.memory.main = wasmer::MemoryType::new(
                        mem.initial as u32,
                        mem.maximum.or(Some(16 * mem.initial)).map(|v| v as u32),
                        true,
                    );
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
        parse_unit_functions(&dwarf, &unit, &mut info)?;
        parse_unit_lines(&dwarf, &unit, &mut info, &mut file_map)?;
    }

    Ok(info)
}

// ============================================================================
// .debug_info: functions, variables, locations
// ============================================================================

fn parse_unit_functions<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    info: &mut DebugInfo,
) -> Result<(), gimli::Error> {
    let mut tree = unit.entries_tree(None)?;
    let root = tree.root()?;
    let mut children = root.children();

    while let Some(child) = children.next()? {
        if child.entry().tag() != gimli::DW_TAG_subprogram {
            continue;
        }

        let name = get_die_name(dwarf, unit, child.entry());
        let pc_range = get_pc_range(child.entry());
        let frame_base = parse_frame_base(child.entry(), unit.encoding());

        let Some(name) = name else { continue };
        let Some((low_pc, high_pc)) = pc_range else { continue };

        let mut variables = Vec::new();
        let mut sub_children = child.children();
        while let Some(var_node) = sub_children.next()? {
            collect_variables(dwarf, unit, var_node, low_pc, high_pc, &mut variables)?;
        }

        let frame_size = compute_frame_layout(&mut variables);

        info.functions.push(DebugFunction {
            name,
            address: low_pc as usize,
            variables,
            frame_size,
            frame_base,
        });
    }

    Ok(())
}

/// Recursively collect variables from a DIE node.
/// Handles `DW_TAG_variable`, `DW_TAG_formal_parameter`, and descends into
/// `DW_TAG_lexical_block` with narrowed scope ranges.
fn collect_variables<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    node: gimli::EntriesTreeNode<'_, '_, R>,
    scope_start: u64,
    scope_end: u64,
    variables: &mut Vec<DebugVariable>,
) -> Result<(), gimli::Error> {
    let tag = node.entry().tag();

    match tag {
        gimli::DW_TAG_variable | gimli::DW_TAG_formal_parameter => {
            let name = get_die_name(dwarf, unit, node.entry());
            let location =
                parse_var_location(dwarf, unit, node.entry(), scope_start, scope_end)?;

            if let Some(name) = name {
                if !location.is_empty() {
                    variables.push(DebugVariable {
                        name,
                        ty: 0,
                        frame_offset: 0,
                        location,
                    });
                }
            }
        }
        gimli::DW_TAG_lexical_block => {
            let (block_start, block_end) =
                get_pc_range(node.entry()).unwrap_or((scope_start, scope_end));

            let mut children = node.children();
            while let Some(child) = children.next()? {
                collect_variables(dwarf, unit, child, block_start, block_end, variables)?;
            }
        }
        _ => {}
    }

    Ok(())
}

// ============================================================================
// DIE attribute helpers
// ============================================================================

fn get_die_name<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    entry: &gimli::DebuggingInformationEntry<R>,
) -> Option<String> {
    let attr = entry.attr(gimli::DW_AT_name)?;
    let s = dwarf.attr_string(unit, attr.value()).ok()?;
    Some(s.to_string_lossy().ok()?.into_owned())
}

fn get_pc_range<R: Reader>(
    entry: &gimli::DebuggingInformationEntry<R>,
) -> Option<(u64, u64)> {
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

fn parse_frame_base<R: Reader>(
    entry: &gimli::DebuggingInformationEntry<R>,
    encoding: gimli::Encoding,
) -> Vec<DwarfOp> {
    let Some(attr) = entry.attr(gimli::DW_AT_frame_base) else {
        return vec![];
    };
    match attr.value() {
        gimli::AttributeValue::Exprloc(expr) => convert_expression(expr, encoding),
        _ => vec![],
    }
}

/// Parse a variable's `DW_AT_location` into location ranges.
/// Handles both simple expressions (`Exprloc`) and location lists (`LocationListsRef`).
fn parse_var_location<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    entry: &gimli::DebuggingInformationEntry<R>,
    default_start: u64,
    default_end: u64,
) -> Result<Vec<VarLocationRange>, gimli::Error> {
    let Some(attr) = entry.attr(gimli::DW_AT_location) else {
        return Ok(vec![]);
    };

    match attr.value() {
        gimli::AttributeValue::Exprloc(expr) => {
            let ops = convert_expression(expr, unit.encoding());
            if ops.is_empty() {
                return Ok(vec![]);
            }
            Ok(vec![VarLocationRange {
                start: default_start,
                end: default_end,
                ops,
            }])
        }
        gimli::AttributeValue::LocationListsRef(offset) => {
            let mut ranges = Vec::new();
            let mut locations = dwarf.locations(unit, offset)?;
            while let Some(entry) = locations.next()? {
                let ops = convert_expression(entry.data, unit.encoding());
                if ops.is_empty() {
                    continue;
                }
                ranges.push(VarLocationRange {
                    start: entry.range.begin,
                    end: entry.range.end,
                    ops,
                });
            }
            Ok(ranges)
        }
        _ => Ok(vec![]),
    }
}

// ============================================================================
// DWARF expression → DwarfOp conversion
// ============================================================================

/// Convert a DWARF expression into owned `DwarfOp` values.
/// Returns empty vec if any unsupported operation is encountered
/// (an incomplete expression is not semantically valid).
fn convert_expression<R: Reader>(expr: gimli::Expression<R>, encoding: gimli::Encoding) -> Vec<DwarfOp> {
    let mut ops = Vec::new();
    let mut iter = expr.operations(encoding);
    loop {
        match iter.next() {
            Ok(Some(op)) => match op {
                gimli::Operation::FrameOffset { offset } => {
                    ops.push(DwarfOp::FrameOffset { offset });
                }
                gimli::Operation::WasmLocal { index } => {
                    ops.push(DwarfOp::WasmLocal { index });
                }
                gimli::Operation::StackValue => {
                    ops.push(DwarfOp::StackValue);
                }
                _ => return vec![],
            },
            Ok(None) => break,
            Err(_) => return vec![],
        }
    }
    ops
}

// ============================================================================
// Debug stack frame layout
// ============================================================================

/// Assign `frame_offset` to each variable and return total frame size.
/// All variables get 4 bytes (i32 width) for the MVP.
fn compute_frame_layout(variables: &mut [DebugVariable]) -> u32 {
    let mut offset = 0u32;
    for var in variables.iter_mut() {
        var.frame_offset = offset;
        offset += 4;
    }
    offset
}

// ============================================================================
// .debug_line: breakpoint locations
// ============================================================================

fn parse_unit_lines<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    info: &mut DebugInfo,
    file_map: &mut HashMap<String, u32>,
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
