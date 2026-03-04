use crate::types::{DebugFrameEntry, DebugFunction, DebugInfo};
use std::collections::HashMap;
use wasm_encoder::{Instruction, MemArg, reencode};

// ============================================================================
// WASM Instrumentation
// ============================================================================

struct Instrumenter<'a> {
    info: &'a mut DebugInfo,
    bkpt_type_index: u32,
    bkpt_fn_index: u32,
    stack_mem_index: u32,
    sp_gl_index: u32,

    num_imported_functions: u32,
    num_imported_globals: u32,

    code_section_start: usize,

    /// Map from code-section byte offset to breakpoint index (1-based; 0 is sentinel).
    breakpoints: HashMap<usize, u32>,
}

impl<'a> Instrumenter<'a> {
    fn new(info: &'a mut DebugInfo) -> Self {
        let breakpoints: HashMap<usize, u32> = info
            .locations
            .iter()
            .enumerate()
            .map(|(i, loc)| (loc.address, i as u32))
            .collect();
        Self {
            info,
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
    fn code_ofs(&self, address: usize) -> usize {
        address.saturating_sub(self.code_section_start)
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
    type Error = core::convert::Infallible;

    fn parse_memory_section(
        &mut self,
        _memories: &mut wasm_encoder::MemorySection,
        _section: wasmparser::MemorySectionReader<'_>,
    ) -> Result<(), reencode::Error> {
        // Note: The instrumented code has no defined memories,
        // as we will be passing the program memory in via import to share it
        Ok(())
    }

    fn function_index(&mut self, func: u32) -> Result<u32, reencode::Error> {
        Ok(if func >= self.num_imported_functions {
            func + 1
        } else {
            func
        })
    }

    fn global_index(&mut self, global: u32) -> Result<u32, reencode::Error> {
        Ok(if global >= self.num_imported_globals {
            global + 1
        } else {
            global
        })
    }

    fn parse_code_section(
        &mut self,
        code: &mut wasm_encoder::CodeSection,
        section: wasmparser::CodeSectionReader<'_>,
    ) -> Result<(), reencode::Error> {
        self.code_section_start = section.range().start;
        reencode::utils::parse_code_section(self, code, section)
    }

    fn parse_type_section(
        &mut self,
        types: &mut wasm_encoder::TypeSection,
        section: wasmparser::TypeSectionReader<'_>,
    ) -> Result<(), reencode::Error> {
        reencode::utils::parse_type_section(self, types, section)?;
        types.ty().function([wasm_encoder::ValType::I32], []);
        self.bkpt_type_index = types.len() - 1;
        Ok(())
    }

    fn parse_import_section(
        &mut self,
        imports: &mut wasm_encoder::ImportSection,
        section: wasmparser::ImportSectionReader<'_>,
    ) -> Result<(), reencode::Error> {
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

        add_mem_import(imports, "memory", &self.info.memory.main);
        add_mem_import(imports, "stack", &self.info.memory.debug);

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
    ) -> Result<(), reencode::Error> {
        /* Get the debug function entry for this function based on its address */
        let body_start = func.range().start;
        let code_ofs = self.code_ofs(body_start);
        let debug_func_idx = self
            .info
            .functions
            .iter()
            .position(|f| f.address == code_ofs);

        let Some(debug_func_idx) = debug_func_idx else {
            // If this is not a function with a corresponding DWARF entry,
            // then we will not do any instrumentation on it and will just emit it as-is.
            return reencode::utils::parse_function_body(self, code, func);
        };

        code.function(&FnInstrumenter::new(self, debug_func_idx).instrument(&func)?);

        Ok(())
    }
}

struct FnInstrumenter<'a, 'b> {
    instr: &'a mut Instrumenter<'b>,
    debug_func_idx: usize,
}

impl<'a, 'b> FnInstrumenter<'a, 'b> {
    fn new(instr: &'a mut Instrumenter<'b>, debug_func_idx: usize) -> Self {
        Self {
            instr,
            debug_func_idx,
        }
    }

    fn debug_func(&mut self) -> &mut DebugFunction {
        &mut self.instr.info.functions[self.debug_func_idx]
    }

    /// Emits the function header, which creates a stack frame on the debug stack.
    fn emit_header(&mut self, f: &mut wasm_encoder::Function) {
        let frame_size = self.debug_func().frame.size;
        f.instructions()
            .global_get(self.instr.sp_gl_index)
            .i32_const(frame_size as i32)
            .i32_sub()
            .global_set(self.instr.sp_gl_index)
            .global_get(self.instr.sp_gl_index)
            .i32_const(self.debug_func_idx as i32)
            .i32_store(MemArg {
                offset: 0,
                align: 2,
                memory_index: self.instr.stack_mem_index,
            });
    }

    /// Emits logic to pause on breakpoints.
    /// Does *not* emit instructions to record values into the debug stack.
    fn emit_bkpt(&mut self, f: &mut wasm_encoder::Function, bkpt_idx: u32) {
        f.instruction(&Instruction::I32Const(bkpt_idx as i32));
        f.instruction(&Instruction::Call(self.instr.bkpt_fn_index));
    }

    /// Emits the function footer, which removes the debug stack frame.
    fn emit_footer(&mut self, f: &mut wasm_encoder::Function) {
        let frame_size = self.debug_func().frame.size;
        f.instructions()
            .global_get(self.instr.sp_gl_index)
            .i32_const(frame_size as i32)
            .i32_add()
            .global_set(self.instr.sp_gl_index);
    }

    fn instrument(
        &mut self,
        func: &wasmparser::FunctionBody<'_>,
    ) -> Result<wasm_encoder::Function, reencode::Error> {
        let mut f = reencode::utils::new_function_with_parsed_locals(self.instr, &func)?;

        self.emit_header(&mut f);

        let mut reader = func.get_operators_reader().map_err(reencode::Error::from)?;
        let body_rel_start = self.instr.code_ofs(func.range().start);
        let first_instr_rel = self.instr.code_ofs(reader.original_position());

        // DWARF addresses that point into the function preamble (body_size + locals)
        // should fire at the first instruction.
        for code_ofs in body_rel_start..first_instr_rel {
            let Some(bkpt_idx) = self.instr.breakpoints.get(&code_ofs).copied() else {
                continue;
            };

            self.emit_bkpt(&mut f, bkpt_idx);
        }

        while !reader.eof() {
            let (op, mut pos) = reader.read_with_offset().map_err(reencode::Error::from)?;
            pos = self.instr.code_ofs(pos);

            if let Some(&bkpt_idx) = self.instr.breakpoints.get(&pos) {
                self.emit_bkpt(&mut f, bkpt_idx);
            }

            // Emit function footer code on return.
            // Note that tail-call instructions must be unwrapped to ensure that we call the
            // footer code at some point.
            match op {
                wasmparser::Operator::Return => {
                    self.emit_footer(&mut f);
                    f.instruction(&Instruction::Return);
                }
                wasmparser::Operator::ReturnCall { function_index } => {
                    f.instruction(&Instruction::Call(function_index));
                    self.emit_footer(&mut f);
                    f.instruction(&Instruction::Return);
                }
                wasmparser::Operator::ReturnCallIndirect {
                    type_index,
                    table_index,
                } => {
                    f.instruction(&Instruction::CallIndirect {
                        type_index,
                        table_index,
                    });
                    self.emit_footer(&mut f);
                    f.instruction(&Instruction::Return);
                }
                wasmparser::Operator::ReturnCallRef { type_index } => {
                    f.instruction(&Instruction::CallRef(type_index));
                    self.emit_footer(&mut f);
                    f.instruction(&Instruction::Return);
                }
                wasmparser::Operator::End => {
                    if reader.eof() {
                        // If this is the final `end` of the function, emit the footer as well
                        self.emit_footer(&mut f);
                    }

                    f.instruction(&reencode::Reencode::instruction(self.instr, op)?);
                }
                _ => {
                    f.instruction(&reencode::Reencode::instruction(self.instr, op)?);
                }
            }
        }

        reader.finish()?;
        Ok(f)
    }
}

/// Instrument a WASM binary by inserting `bkpt` calls at DWARF line boundaries.
///
/// Adds import: `(import "debug" "bkpt" (func (param i32)))`
/// The i32 param is the breakpoint index (1-based, 0 is sentinel).
pub fn instrument_wasm(wasm_bytes: &[u8], debug_info: &mut DebugInfo) -> Result<Vec<u8>, String> {
    let mut instrumenter = Instrumenter::new(debug_info);
    let mut module = wasm_encoder::Module::new();
    reencode::utils::parse_core_module(
        &mut instrumenter,
        &mut module,
        wasmparser::Parser::new(0),
        wasm_bytes,
    )
    .map_err(|e| format!("Failed to reencode WASM: {:?}", e))?;
    Ok(module.finish())
}
