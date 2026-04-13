use std::collections::HashMap;

use super::{Error, FnInstrumenter, InstrResult};
use crate::types::{DebugInfo, GlobalAddress};
use wasm_encoder::reencode::{self};

pub struct Instrumenter<'a> {
    pub info: &'a mut DebugInfo,
    pub validator: wasmparser::Validator,
    pub bkpt_type_index: u32,
    pub bkpt_fn_index: u32,
    pub stack_mem_index: u32,
    pub sp_gl_index: u32,

    pub num_imported_functions: u32,
    pub num_imported_globals: u32,

    pub code_section_start: usize,

    /// Map from code-section byte offset to breakpoint index (1-based; 0 is sentinel).
    pub breakpoints: std::collections::HashMap<GlobalAddress, usize>,
}

impl<'a> Instrumenter<'a> {
    pub fn new(info: &'a mut DebugInfo) -> Self {
        let mut breakpoints = HashMap::new();
        for (index, loc) in info.dwarf.locations().enumerate() {
            breakpoints.entry(loc.address()).or_insert(index);
        }

        Self {
            info,
            validator: wasmparser::Validator::new(),
            bkpt_type_index: 0,
            bkpt_fn_index: 0,
            stack_mem_index: 1,
            sp_gl_index: 0,
            num_imported_functions: 0,
            num_imported_globals: 0,
            code_section_start: 0,
            breakpoints,
        }
    }

    /// Converts an offset into the WASM binary into an offset relative to the code section.
    /// DWARF represents PC values relative to start of the code section.
    pub fn code_ofs(&self, address: usize) -> GlobalAddress {
        GlobalAddress(address.saturating_sub(self.code_section_start) as u64)
    }
}

fn count_imports(
    imports: &wasmparser::Imports<'_>,
    pred: impl Fn(&wasmparser::TypeRef) -> bool,
) -> u32 {
    match imports {
        wasmparser::Imports::Single(_, import) => pred(&import.ty) as u32,
        wasmparser::Imports::Compact1 { items, .. } => items
            .clone()
            .into_iter()
            .filter(|item| item.as_ref().map_or(false, |i| pred(&i.ty)))
            .count() as u32,
        wasmparser::Imports::Compact2 { ty, names, .. } => {
            if pred(ty) {
                names.count()
            } else {
                0
            }
        }
    }
}

fn count_function_imports(imports: &wasmparser::Imports<'_>) -> u32 {
    use wasmparser::TypeRef;
    count_imports(imports, |ty| {
        matches!(ty, TypeRef::Func(_) | TypeRef::FuncExact(_))
    })
}

fn count_global_imports(imports: &wasmparser::Imports<'_>) -> u32 {
    use wasmparser::TypeRef;
    count_imports(imports, |ty| matches!(ty, TypeRef::Global(_)))
}

impl<'a> reencode::Reencode for Instrumenter<'a> {
    type Error = Error;

    fn function_index(&mut self, func: u32) -> InstrResult<u32> {
        Ok(if func >= self.num_imported_functions {
            func + 1
        } else {
            func
        })
    }

    fn global_index(&mut self, global: u32) -> InstrResult<u32> {
        Ok(if global >= self.num_imported_globals {
            global + 1
        } else {
            global
        })
    }

    fn parse_global_section(
        &mut self,
        globals: &mut wasm_encoder::GlobalSection,
        section: wasmparser::GlobalSectionReader<'_>,
    ) -> InstrResult {
        self.validator
            .global_section(&section)
            .map_err(reencode::Error::from)?;
        reencode::utils::parse_global_section(self, globals, section)
    }

    fn parse_memory_section(
        &mut self,
        _memories: &mut wasm_encoder::MemorySection,
        section: wasmparser::MemorySectionReader<'_>,
    ) -> InstrResult {
        self.validator
            .memory_section(&section)
            .map_err(reencode::Error::from)?;

        // Note: The instrumented code has no defined memories,
        // as we will be passing the program memory in via import to share it
        Ok(())
    }

    fn parse_function_section(
        &mut self,
        functions: &mut wasm_encoder::FunctionSection,
        section: wasmparser::FunctionSectionReader<'_>,
    ) -> InstrResult {
        self.validator
            .function_section(&section)
            .map_err(reencode::Error::from)?;
        reencode::utils::parse_function_section(self, functions, section)
    }

    fn parse_code_section(
        &mut self,
        code: &mut wasm_encoder::CodeSection,
        section: wasmparser::CodeSectionReader<'_>,
    ) -> InstrResult {
        self.code_section_start = section.range().start;
        self.validator
            .code_section_start(&section.range())
            .map_err(reencode::Error::from)?;
        reencode::utils::parse_code_section(self, code, section)
    }

    fn parse_type_section(
        &mut self,
        types: &mut wasm_encoder::TypeSection,
        section: wasmparser::TypeSectionReader<'_>,
    ) -> InstrResult {
        self.validator
            .version(1, wasmparser::Encoding::Module, &(0..8))
            .map_err(reencode::Error::from)?;
        self.validator
            .type_section(&section)
            .map_err(reencode::Error::from)?;
        reencode::utils::parse_type_section(self, types, section)?;
        types.ty().function([wasm_encoder::ValType::I32], []);
        self.bkpt_type_index = types.len() - 1;
        Ok(())
    }

    fn parse_import_section(
        &mut self,
        imports: &mut wasm_encoder::ImportSection,
        section: wasmparser::ImportSectionReader<'_>,
    ) -> InstrResult {
        self.validator
            .import_section(&section)
            .map_err(reencode::Error::from)?;

        for batch in section {
            let batch = batch?;
            self.num_imported_functions += count_function_imports(&batch);
            self.num_imported_globals += count_global_imports(&batch);
            reencode::utils::parse_imports(self, imports, batch)?;
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

        add_mem_import(imports, "memory", &self.info.memory.ty);
        add_mem_import(imports, "stack", &self.info.stack.ty);

        self.sp_gl_index = self.num_imported_globals;
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
    ) -> InstrResult {
        /* Get the debug function entry for this function based on its address */
        let body_start = func.range().start;
        let code_ofs = self.code_ofs(body_start);
        let debug_func_idx = self.info.fn_index_at(code_ofs.into());

        let Some(debug_func_idx) = debug_func_idx else {
            // If this is not a function with a corresponding DWARF entry,
            // then we will not do any instrumentation on it and will just emit it as-is.
            //
            // Note that `wasmparser` still needs us to process this function,
            // even if we do any validation with it
            self.validator.code_section_entry(&func)?;
            return reencode::utils::parse_function_body::<Self>(self, code, func);
        };

        let fn_instr = FnInstrumenter::new(self, debug_func_idx, func)?;
        code.function(&fn_instr.instrument()?);

        Ok(())
    }

    fn parse_table_section(
        &mut self,
        tables: &mut wasm_encoder::TableSection,
        section: wasmparser::TableSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        self.validator
            .table_section(&section)
            .map_err(reencode::Error::from)?;
        reencode::utils::parse_table_section(self, tables, section)
    }
}
