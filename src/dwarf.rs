use crate::types::LocationInfo;
use gimli::{EndianSlice, LittleEndian, Reader};
use object::{Object, ObjectSection};
use std::borrow::Cow;
use std::collections::HashMap;
use wasmer_wasix::virtual_fs::{AsyncReadExt, FileSystem, mem_fs};

/// ============================================================================
/// HELPERS
/// ============================================================================

/// Get the WASM bytes from the filesystem.
/// Returns the WASM bytes or an error if the file does not exist.
pub async fn get_wasm_bytes(
    fs: &mem_fs::FileSystem,
    path: &str,
) -> Result<Vec<u8>, std::io::Error> {
    let mut file = fs
        .new_open_options()
        .read(true)
        .open(path)
        .expect(&format!("{} exists", path));

    let mut wasm_bytes = Vec::new();
    file.read_to_end(&mut wasm_bytes)
        .await
        .expect("Read main.wasm");

    Ok(wasm_bytes)
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

/// ============================================================================
/// DWARF PARSING
/// ============================================================================

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
    let dwarf =
        dwarf_sections.borrow(|section| EndianSlice::new(Cow::as_ref(section), LittleEndian));

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
                address: row.address(),
            });
        }
    }

    Ok((locations, files))
}

/// ============================================================================
/// WASM INSTRUMENTATION
/// ============================================================================

/// Instrument a WASM binary by inserting `bkpt` calls at line boundaries.
///
/// Adds import: `(import "debug" "bkpt" (func (param i32)))`
/// The i32 param is the breakpoint index (1-based, 0 is sentinel).
pub fn instrument_binary(
    wasm_bytes: &[u8],
    _locations: &[LocationInfo],
) -> Result<Vec<u8>, String> {
    use wasm_encoder::reencode::{Error as ReencodeError, Reencode};
    use wasm_encoder::*;

    // First pass: count types and imported functions so we know the indices
    // for the new type and the new import we're about to add.
    let mut num_types = 0u32;
    let mut num_imported_funcs = 0u32;

    for payload in wasmparser::Parser::new(0).parse_all(wasm_bytes) {
        let payload = payload.map_err(|e| format!("Parse error: {e}"))?;
        match payload {
            wasmparser::Payload::TypeSection(reader) => {
                for rec_group in reader {
                    rec_group.map_err(|e| format!("Type error: {e}"))?;
                    num_types += 1;
                }
            }
            wasmparser::Payload::ImportSection(reader) => {
                for import in reader {
                    let import = import.map_err(|e| format!("Import error: {e}"))?;
                    if matches!(import.ty, wasmparser::TypeRef::Func(_)) {
                        num_imported_funcs += 1;
                    }
                }
            }
            _ => {}
        }
    }

    // Custom reencoder: appends a bkpt type + import, and shifts all local
    // function indices by 1 to account for the newly-inserted import.
    struct BkptReencoder {
        num_types: u32,
        num_imported_funcs: u32,
    }

    impl Reencode for BkptReencoder {
        type Error = std::convert::Infallible;

        fn function_index(&mut self, func: u32) -> u32 {
            if func >= self.num_imported_funcs {
                func + 1
            } else {
                func
            }
        }

        fn parse_type_section(
            &mut self,
            types: &mut TypeSection,
            section: wasmparser::TypeSectionReader<'_>,
        ) -> Result<(), ReencodeError<Self::Error>> {
            for rec_group in section {
                self.parse_recursive_type_group(types.ty(), rec_group?)?;
            }
            // Append: (func (param i32))
            types.ty().function(vec![ValType::I32], vec![]);
            Ok(())
        }

        fn parse_import_section(
            &mut self,
            imports: &mut ImportSection,
            section: wasmparser::ImportSectionReader<'_>,
        ) -> Result<(), ReencodeError<Self::Error>> {
            for import in section {
                self.parse_import(imports, import?)?;
            }
            // Append: (import "debug" "bkpt" (func <new_type_idx>))
            imports.import("debug", "bkpt", EntityType::Function(self.num_types));
            Ok(())
        }

        fn parse_custom_section(
            &mut self,
            module: &mut Module,
            section: wasmparser::CustomSectionReader<'_>,
        ) -> Result<(), ReencodeError<Self::Error>> {
            if section.name().starts_with("reloc..") {
                return Ok(());
            }

            let wasmparser::KnownCustom::Linking(reader) = section.as_known() else {
                return wasm_encoder::reencode::utils::parse_custom_section(self, module, section);
            };

            let mut linking = LinkingSection::new();
            let mut sym_tab = SymbolTable::new();

            for subsection in reader {
                if let wasmparser::Linking::SymbolTable(symbols) = subsection? {
                    for sym in symbols {
                        match sym? {
                            wasmparser::SymbolInfo::Func { flags, index, name } => {
                                sym_tab.function(flags.bits(), self.function_index(index), name);
                            }
                            wasmparser::SymbolInfo::Data {
                                flags,
                                name,
                                symbol,
                            } => {
                                sym_tab.data(
                                    flags.bits(),
                                    name,
                                    symbol.map(|s| DataSymbolDefinition {
                                        index: s.index,
                                        offset: s.offset,
                                        size: s.size,
                                    }),
                                );
                            }
                            wasmparser::SymbolInfo::Global { flags, index, name } => {
                                sym_tab.global(flags.bits(), index, name);
                            }
                            wasmparser::SymbolInfo::Table { flags, index, name } => {
                                sym_tab.table(flags.bits(), index, name);
                            }
                            // Section and Event symbols not yet supported by
                            // wasm-encoder's SymbolTable — skip for now.
                            _ => {}
                        }
                    }
                }
            }

            linking.symbol_table(&sym_tab);
            module.section(&linking);
            Ok(())
        }
    }

    let mut reencoder = BkptReencoder {
        num_types,
        num_imported_funcs,
    };

    let mut module = Module::new();
    reencoder
        .parse_core_module(&mut module, wasmparser::Parser::new(0), wasm_bytes)
        .map_err(|e| format!("Reencode error: {e:?}"))?;

    Ok(module.finish())
}
