use crate::types::DebugInfo;
use std::collections::HashMap;
use wasm_encoder::Instruction;

// ============================================================================
// WASM Instrumentation
// ============================================================================

struct Instrumenter<'a> {
    info: &'a DebugInfo,
    bkpt_type_index: u32,
    bkpt_fn_index: u32,
    num_imported_functions: u32,
    code_section_start: usize,
    /// Map from code-section byte offset to breakpoint index (1-based; 0 is sentinel).
    breakpoints: HashMap<u64, u32>,
}

impl<'a> Instrumenter<'a> {
    fn new(info: &'a DebugInfo) -> Self {
        let breakpoints: HashMap<u64, u32> = info
            .locations
            .iter()
            .enumerate()
            .map(|(i, loc)| (loc.address, i as u32))
            .collect();
        Self {
            info,
            bkpt_type_index: 0,
            bkpt_fn_index: 0,
            num_imported_functions: 0,
            code_section_start: 0,
            breakpoints,
        }
    }
}

fn count_function_imports(imports: &wasmparser::Imports<'_>) -> u32 {
    use wasmparser::TypeRef;
    match imports {
        wasmparser::Imports::Single(_, import) => {
            matches!(import.ty, TypeRef::Func(_) | TypeRef::FuncExact(_)) as u32
        }
        wasmparser::Imports::Compact1 { items, .. } => items
            .clone()
            .into_iter()
            .filter(|item| {
                item.as_ref().map_or(false, |i| {
                    matches!(i.ty, TypeRef::Func(_) | TypeRef::FuncExact(_))
                })
            })
            .count() as u32,
        wasmparser::Imports::Compact2 { ty, names, .. } => {
            if matches!(ty, TypeRef::Func(_) | TypeRef::FuncExact(_)) {
                names.count()
            } else {
                0
            }
        }
    }
}

impl<'a> wasm_encoder::reencode::Reencode for Instrumenter<'a> {
    type Error = core::convert::Infallible;

    fn parse_memory_section(
        &mut self,
        _memories: &mut wasm_encoder::MemorySection,
        _section: wasmparser::MemorySectionReader<'_>,
    ) -> Result<(), wasm_encoder::reencode::Error<Self::Error>> {
        // Note: The instrumented code has no defined memories,
        // as we will be passing the program memory in via import to share it
        Ok(())
    }

    fn function_index(
        &mut self,
        func: u32,
    ) -> Result<u32, wasm_encoder::reencode::Error<Self::Error>> {
        Ok(if func >= self.num_imported_functions {
            func + 1
        } else {
            func
        })
    }

    fn parse_code_section(
        &mut self,
        code: &mut wasm_encoder::CodeSection,
        section: wasmparser::CodeSectionReader<'_>,
    ) -> Result<(), wasm_encoder::reencode::Error<Self::Error>> {
        self.code_section_start = section.range().start;
        wasm_encoder::reencode::utils::parse_code_section(self, code, section)
    }

    fn parse_type_section(
        &mut self,
        types: &mut wasm_encoder::TypeSection,
        section: wasmparser::TypeSectionReader<'_>,
    ) -> Result<(), wasm_encoder::reencode::Error<Self::Error>> {
        wasm_encoder::reencode::utils::parse_type_section(self, types, section)?;
        types.ty().function([wasm_encoder::ValType::I32], []);
        self.bkpt_type_index = types.len() - 1;
        Ok(())
    }

    fn parse_import_section(
        &mut self,
        imports: &mut wasm_encoder::ImportSection,
        section: wasmparser::ImportSectionReader<'_>,
    ) -> Result<(), wasm_encoder::reencode::Error<Self::Error>> {
        self.num_imported_functions = 0u32;
        for batch in section {
            let batch = batch?;
            self.num_imported_functions += count_function_imports(&batch);
            wasm_encoder::reencode::utils::parse_imports(self, imports, batch)?;
        }

        self.bkpt_fn_index = self.num_imported_functions;
        imports.import(
            "debug",
            "bkpt",
            wasm_encoder::EntityType::Function(self.bkpt_type_index),
        );

        fn add_mem_import(
            imports: &mut wasm_encoder::ImportSection,
            name: &str,
            memory: &wasmer::MemoryType,
        ) {
            imports.import(
                "debug",
                name,
                wasm_encoder::EntityType::Memory(wasm_encoder::MemoryType {
                    minimum: memory.minimum.0 as u64,
                    maximum: memory.maximum.and_then(|v| Some(v.0 as u64)),
                    memory64: false,
                    shared: memory.shared,
                    page_size_log2: None,
                }),
            );
        }

        add_mem_import(imports, "memory", &self.info.memory.main);
        add_mem_import(imports, "stack", &self.info.memory.debug);

        // TODO: Bug. This might end up shifting other imports in the module,
        // necesitating overriding `import_index`
        //
        // This is technically true of the memories as well, although we make
        // the assumption that an inputted binary will only have one (defined) memory.
        imports.import(
            "debug",
            "sp",
            wasm_encoder::EntityType::Global(wasm_encoder::GlobalType {
                val_type: wasm_encoder::ValType::I32,
                mutable: true,
                shared: false,
            }),
        );

        Ok(())
    }

    fn parse_function_body(
        &mut self,
        code: &mut wasm_encoder::CodeSection,
        func: wasmparser::FunctionBody<'_>,
    ) -> Result<(), wasm_encoder::reencode::Error<Self::Error>> {
        let mut f = wasm_encoder::reencode::utils::new_function_with_parsed_locals(self, &func)?;
        let mut reader = func
            .get_operators_reader()
            .map_err(wasm_encoder::reencode::Error::from)?;

        let body_rel_start = func.range().start.saturating_sub(self.code_section_start) as u64;
        let first_instr_rel = reader
            .original_position()
            .saturating_sub(self.code_section_start) as u64;

        // DWARF addresses that point into the function preamble (body_size + locals)
        // should fire at the first instruction.
        for code_ofs in body_rel_start..first_instr_rel {
            let Some(bkpt_idx) = self.breakpoints.get(&code_ofs).copied() else {
                continue;
            };

            f.instruction(&Instruction::I32Const(bkpt_idx as i32));
            f.instruction(&Instruction::Call(self.bkpt_fn_index));
        }

        while !reader.eof() {
            let (op, pos) = reader
                .read_with_offset()
                .map_err(wasm_encoder::reencode::Error::from)?;

            let code_offset = pos.saturating_sub(self.code_section_start) as u64;
            if let Some(&idx) = self.breakpoints.get(&code_offset) {
                f.instruction(&Instruction::I32Const(idx as i32));
                f.instruction(&Instruction::Call(self.bkpt_fn_index));
            }

            let insn = self.instruction(op)?;
            f.instruction(&insn);
        }

        reader.finish()?;
        code.function(&f);
        Ok(())
    }
}

/// Instrument a WASM binary by inserting `bkpt` calls at DWARF line boundaries.
///
/// Adds import: `(import "debug" "bkpt" (func (param i32)))`
/// The i32 param is the breakpoint index (1-based, 0 is sentinel).
pub fn instrument_wasm(wasm_bytes: &[u8], debug_info: &DebugInfo) -> Result<Vec<u8>, String> {
    let mut instrumenter = Instrumenter::new(debug_info);
    let mut module = wasm_encoder::Module::new();
    wasm_encoder::reencode::utils::parse_core_module(
        &mut instrumenter,
        &mut module,
        wasmparser::Parser::new(0),
        wasm_bytes,
    )
    .map_err(|e| format!("Failed to reencode WASM: {:?}", e))?;
    Ok(module.finish())
}
