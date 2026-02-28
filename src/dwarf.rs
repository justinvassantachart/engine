use crate::types::{
    DebugFunction, DebugInfo, DebugType, DebugVariable, DwarfOp, LocationInfo, TypeEncoding,
    VarLocationRange,
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

        // Pass 1: flat scan to collect all type DIEs (base_type, pointer, struct, etc.)
        // and record alias chains (typedef, const, volatile, restrict).
        let mut type_map = HashMap::new();
        parse_unit_types(&dwarf, &unit, &mut info, &mut type_map)?;

        // Pass 2: tree walk for functions/variables, resolving DW_AT_type via type_map.
        parse_unit_functions(&dwarf, &unit, &mut info, &type_map)?;

        parse_unit_lines(&dwarf, &unit, &mut info, &mut file_map)?;
    }

    Ok(info)
}

// ============================================================================
// .debug_info: type collection (flat pass)
// ============================================================================

type TypeMap<Offset> = HashMap<gimli::UnitOffset<Offset>, u32>;

/// Flat scan of all DIEs in a unit to collect type information.
/// Concrete types (base, pointer, struct, enum, array) → pushed to `info.types`.
/// Alias types (typedef, const, volatile, restrict) → resolved via chain-following.
fn parse_unit_types<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    info: &mut DebugInfo,
    type_map: &mut TypeMap<R::Offset>,
) -> Result<(), gimli::Error> {
    let mut alias_map: HashMap<gimli::UnitOffset<R::Offset>, gimli::UnitOffset<R::Offset>> =
        HashMap::new();

    let mut entries = unit.entries();
    while let Some(entry) = entries.next_dfs()? {
        let offset = entry.offset();

        match entry.tag() {
            gimli::DW_TAG_base_type => {
                let name = get_die_name(dwarf, unit, entry).unwrap_or_default();
                let byte_size = get_byte_size(entry).unwrap_or(0);
                let encoding = get_type_encoding(entry);

                let idx = info.types.len() as u32;
                info.types.push(DebugType {
                    name,
                    byte_size,
                    encoding,
                    offset: 0,
                    fields: vec![],
                });
                type_map.insert(offset, idx);
            }
            gimli::DW_TAG_pointer_type => {
                let byte_size = get_byte_size(entry).unwrap_or(4);
                let idx = info.types.len() as u32;
                info.types.push(DebugType {
                    name: "ptr".into(),
                    byte_size,
                    encoding: TypeEncoding::Address,
                    offset: 0,
                    fields: vec![],
                });
                type_map.insert(offset, idx);
            }
            gimli::DW_TAG_structure_type | gimli::DW_TAG_union_type | gimli::DW_TAG_class_type => {
                let name = get_die_name(dwarf, unit, entry).unwrap_or_else(|| "<anon>".into());
                let byte_size = get_byte_size(entry).unwrap_or(0);
                let idx = info.types.len() as u32;
                info.types.push(DebugType {
                    name,
                    byte_size,
                    encoding: TypeEncoding::Unknown,
                    offset: 0,
                    fields: vec![],
                });
                type_map.insert(offset, idx);
            }
            gimli::DW_TAG_reference_type | gimli::DW_TAG_rvalue_reference_type => {
                let byte_size = get_byte_size(entry).unwrap_or(4);
                let idx = info.types.len() as u32;
                info.types.push(DebugType {
                    name: "ref".into(),
                    byte_size,
                    encoding: TypeEncoding::Address,
                    offset: 0,
                    fields: vec![],
                });
                type_map.insert(offset, idx);
            }
            gimli::DW_TAG_enumeration_type => {
                let name = get_die_name(dwarf, unit, entry).unwrap_or_else(|| "<enum>".into());
                let byte_size = get_byte_size(entry).unwrap_or(4);
                let idx = info.types.len() as u32;
                info.types.push(DebugType {
                    name,
                    byte_size,
                    encoding: TypeEncoding::Unsigned,
                    offset: 0,
                    fields: vec![],
                });
                type_map.insert(offset, idx);
            }
            gimli::DW_TAG_array_type => {
                if let Some(byte_size) = get_byte_size(entry) {
                    let idx = info.types.len() as u32;
                    info.types.push(DebugType {
                        name: "array".into(),
                        byte_size,
                        encoding: TypeEncoding::Unknown,
                        offset: 0,
                        fields: vec![],
                    });
                    type_map.insert(offset, idx);
                }
            }
            gimli::DW_TAG_typedef
            | gimli::DW_TAG_const_type
            | gimli::DW_TAG_volatile_type
            | gimli::DW_TAG_restrict_type => {
                if let Some(target) = get_type_ref(entry) {
                    alias_map.insert(offset, target);
                }
            }
            _ => {}
        }
    }

    // Resolve alias chains: typedef → const → base_type etc.
    let resolved: Vec<_> = alias_map
        .keys()
        .filter_map(|alias| {
            resolve_type_alias(alias, type_map, &alias_map, 0).map(|idx| (*alias, idx))
        })
        .collect();
    for (offset, idx) in resolved {
        type_map.insert(offset, idx);
    }

    Ok(())
}

/// Follow alias chains (typedef → const → base_type) to find the concrete type index.
fn resolve_type_alias<Offset: Copy + Eq + std::hash::Hash>(
    offset: &gimli::UnitOffset<Offset>,
    type_map: &TypeMap<Offset>,
    alias_map: &HashMap<gimli::UnitOffset<Offset>, gimli::UnitOffset<Offset>>,
    depth: u32,
) -> Option<u32> {
    if depth > 16 {
        return None;
    }
    if let Some(&idx) = type_map.get(offset) {
        return Some(idx);
    }
    let target = alias_map.get(offset)?;
    resolve_type_alias(target, type_map, alias_map, depth + 1)
}

// ============================================================================
// .debug_info: functions and variables (tree walk)
// ============================================================================

/// Pass 2: walk the DIE tree and collect subprograms (functions).
/// Recurses into namespace/module/class/struct/union so nested functions are found.
fn parse_unit_functions<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    info: &mut DebugInfo,
    type_map: &TypeMap<R::Offset>,
) -> Result<(), gimli::Error> {
    let mut tree = unit.entries_tree(None)?;
    let root = tree.root()?;
    let mut children = root.children();

    while let Some(child) = children.next()? {
        collect_subprograms_from_node(dwarf, unit, child, info, type_map)?;
    }

    Ok(())
}

/// Recursively find subprogram DIEs and push them to info.functions.
/// Recurse into containers (namespace, module, class, struct, union) to find nested functions.
fn collect_subprograms_from_node<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    node: gimli::EntriesTreeNode<'_, '_, R>,
    info: &mut DebugInfo,
    type_map: &TypeMap<R::Offset>,
) -> Result<(), gimli::Error> {
    let tag = node.entry().tag();

    if tag == gimli::DW_TAG_subprogram {
        let name = get_die_name(dwarf, unit, node.entry());
        let pc_range = get_pc_range(node.entry());
        let frame_base = parse_frame_base(node.entry(), unit.encoding());

        if let (Some(name), Some((low_pc, high_pc))) = (name, pc_range) {
            let mut variables = Vec::new();
            let mut sub_children = node.children();
            while let Some(var_node) = sub_children.next()? {
                collect_variables(
                    dwarf,
                    unit,
                    var_node,
                    low_pc,
                    high_pc,
                    &mut variables,
                    type_map,
                )?;
            }

            let frame_size = compute_frame_layout(&mut variables, &info.types);

            info.functions.push(DebugFunction {
                name,
                address: low_pc as usize,
                variables,
                frame_size,
                frame_base,
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
            collect_subprograms_from_node(dwarf, unit, child, info, type_map)?;
        }
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
    type_map: &TypeMap<R::Offset>,
) -> Result<(), gimli::Error> {
    let tag = node.entry().tag();

    match tag {
        gimli::DW_TAG_variable | gimli::DW_TAG_formal_parameter => {
            let name = get_die_name(dwarf, unit, node.entry());
            let location = parse_var_location(
                dwarf,
                unit,
                node.entry(),
                scope_start as usize,
                scope_end as usize,
            )?;
            let ty = get_type_ref(node.entry())
                .and_then(|offset| type_map.get(&offset).copied())
                .unwrap_or(0);

            if let Some(name) = name {
                if !location.is_empty() {
                    variables.push(DebugVariable {
                        name,
                        ty,
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
                collect_variables(
                    dwarf,
                    unit,
                    child,
                    block_start,
                    block_end,
                    variables,
                    type_map,
                )?;
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

fn get_byte_size<R: Reader>(entry: &gimli::DebuggingInformationEntry<R>) -> Option<u32> {
    entry
        .attr(gimli::DW_AT_byte_size)?
        .udata_value()
        .map(|v| v as u32)
}

fn get_type_encoding<R: Reader>(entry: &gimli::DebuggingInformationEntry<R>) -> TypeEncoding {
    let Some(attr) = entry.attr(gimli::DW_AT_encoding) else {
        return TypeEncoding::Unknown;
    };
    match attr.value() {
        gimli::AttributeValue::Encoding(enc) => match enc {
            gimli::DW_ATE_signed | gimli::DW_ATE_signed_char => TypeEncoding::Signed,
            gimli::DW_ATE_unsigned | gimli::DW_ATE_unsigned_char => TypeEncoding::Unsigned,
            gimli::DW_ATE_float => TypeEncoding::Float,
            gimli::DW_ATE_boolean => TypeEncoding::Bool,
            gimli::DW_ATE_address => TypeEncoding::Address,
            _ => TypeEncoding::Unknown,
        },
        _ => TypeEncoding::Unknown,
    }
}

fn get_type_ref<R: Reader>(
    entry: &gimli::DebuggingInformationEntry<R>,
) -> Option<gimli::UnitOffset<R::Offset>> {
    let attr = entry.attr(gimli::DW_AT_type)?;
    match attr.value() {
        gimli::AttributeValue::UnitRef(offset) => Some(offset),
        _ => None,
    }
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
    default_start: usize,
    default_end: usize,
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
                    start: entry.range.begin as usize,
                    end: entry.range.end as usize,
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
fn convert_expression<R: Reader>(
    expr: gimli::Expression<R>,
    encoding: gimli::Encoding,
) -> Vec<DwarfOp> {
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
/// Each slot is at least 4 bytes (i32 width) to match the `i32.store` used by
/// the instrumenter. Larger types get their full byte_size so the layout is
/// correct for future multi-word copies.
fn compute_frame_layout(variables: &mut [DebugVariable], types: &[DebugType]) -> u32 {
    let mut offset = 0u32;
    for var in variables.iter_mut() {
        let byte_size = types.get(var.ty as usize).map_or(4, |t| t.byte_size.max(4));
        offset = (offset + 3) & !3;
        var.frame_offset = offset;
        offset += byte_size;
    }
    (offset + 3) & !3
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
