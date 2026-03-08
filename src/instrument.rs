use crate::types::{DebugFunction, DebugInfo, DwarfOp, WasmOp};
use std::collections::{BTreeSet, HashMap};
use wasm_encoder::{
    Instruction, MemArg,
    reencode::{self, Reencode},
};
use wasmparser::ValType;

type Error = anyhow::Error;
type InstrError = reencode::Error<Error>;
type InstrResult<T = ()> = Result<T, InstrError>;

macro_rules! error {
    ($($arg:tt)*) => {
        Err(InstrError::UserError(Error::msg(format!($($arg)*))))
    };
}

// ============================================================================
// WASM Instrumentation
// ============================================================================

struct Instrumenter<'a> {
    info: &'a mut DebugInfo,
    validator: wasmparser::Validator,
    bkpt_type_index: u32,
    bkpt_fn_index: u32,
    stack_mem_index: u32,
    sp_gl_index: u32,

    num_imported_functions: u32,
    num_imported_globals: u32,

    code_section_start: usize,

    /// Map from code-section byte offset to breakpoint index (1-based; 0 is sentinel).
    breakpoints: HashMap<usize, usize>,
}

impl<'a> Instrumenter<'a> {
    fn new(info: &'a mut DebugInfo) -> Self {
        let breakpoints: HashMap<usize, usize> = info
            .locations
            .iter()
            .enumerate()
            .map(|(i, loc)| (loc.address, i))
            .collect();
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
    ) -> InstrResult {
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
}

#[derive(Default)]
struct WasmLocations {
    operands: BTreeSet<usize>,
    locals: BTreeSet<usize>,
    globals: BTreeSet<usize>,
}

impl DebugFunction {
    /// Returns the WebAssembly locations needed to evaluate this function's variables at `addr`
    fn wasm_locations_at(&self, addr: usize) -> WasmLocations {
        let mut out = WasmLocations::default();
        let fb = self.frame.base.location_at(addr);
        for var in &self.variables {
            if let Some(range) = var.location.location_at(addr) {
                collect_wasm_locs(&range.ops, fb, &mut out);
            }
        }
        out
    }
}

fn collect_wasm_locs(
    ops: &[DwarfOp],
    fb: Option<&crate::types::VarLocationRange>,
    out: &mut WasmLocations,
) {
    for op in ops {
        match op {
            DwarfOp::Wasm(loc) => {
                match *loc {
                    WasmOp::Global(idx) => out.globals.insert(idx),
                    WasmOp::Local(idx) => out.locals.insert(idx),
                    WasmOp::Operand(idx) => out.operands.insert(idx),
                };
            }
            DwarfOp::FrameOffset { .. } => {
                if let Some(frame_base) = fb {
                    collect_wasm_locs(&frame_base.ops, fb, out);
                }
            }
            _ => {}
        }
    }
}

struct FnInstrumenter<'a, 'b, 'c> {
    instr: &'a mut Instrumenter<'b>,
    debug_func_idx: usize,
    func_body: wasmparser::FunctionBody<'c>,

    /// A [wasmparser] validator used to track the types of locals and operands
    /// throughout the instrumentation of this function
    validator: wasmparser::FuncValidator<wasmparser::ValidatorResources>,

    /// A vector of instructions forming the body of the instrumented function
    instructions: Vec<Instruction<'c>>,

    /// During instrumentation, the size of the function's debug frame is not
    /// yet known. This vector stores indexes into [FnInstrumenter::instructions]
    /// representing `i32.const` instructions which should eventually contain
    /// `i32.const F`, where `F` is the eventual size of this function's stack frame.
    /// Before emitting the instrumented function body, such instructions will be
    /// replaced with the correct frame size.
    stack_intructions: Vec<usize>,

    /// A vector of the types of scratch locals which will be used to store
    /// operand stack values when peeling off the operand stack to recover
    /// a value.
    ///
    /// In the instrumented function, these will follow the function's
    /// own locals. If `N` is the number of original locals
    /// (parameters + additional locals), then each of these will have local
    /// indices `N, N+1, ...`.
    scratch_locals: Vec<wasm_encoder::ValType>,
}

impl<'a, 'b, 'c> FnInstrumenter<'a, 'b, 'c> {
    fn new(
        instr: &'a mut Instrumenter<'b>,
        debug_func_idx: usize,
        func_body: wasmparser::FunctionBody<'c>,
    ) -> InstrResult<Self> {
        let mut validator = instr
            .validator
            .code_section_entry(&func_body)
            .map_err(reencode::Error::from)?
            .into_validator(Default::default());

        validator.read_locals(&mut func_body.get_binary_reader())?;

        Ok(Self {
            instr,
            debug_func_idx,
            func_body,
            validator,

            instructions: Vec::default(),
            stack_intructions: Vec::default(),
            scratch_locals: Vec::default(),
        })
    }

    fn debug_func(&mut self) -> &mut DebugFunction {
        &mut self.instr.info.functions[self.debug_func_idx]
    }

    fn emit_header(&mut self) {
        let instr_count = self.instructions.len();
        let frame_size = self.debug_func().frame.size;
        self.instructions.extend([
            Instruction::GlobalGet(self.instr.sp_gl_index),
            Instruction::I32Const(frame_size as i32),
            Instruction::I32Sub,
            Instruction::GlobalSet(self.instr.sp_gl_index),
            Instruction::GlobalGet(self.instr.sp_gl_index),
            Instruction::I32Const(self.debug_func_idx as i32),
            Instruction::I32Store(MemArg {
                offset: 0,
                align: 2,
                memory_index: self.instr.stack_mem_index,
            }),
        ]);
        self.stack_intructions.push(instr_count + 1);
    }

    fn emit_bkpt(&mut self, bkpt_idx: usize) -> InstrResult {
        // High-level goal:
        // Loop through all variables of the function.
        // For every variable with an active location at this point in the
        // program, insert instrumentation code to store the WASM internals
        // needed to derive the variable's value at runtime onto the debug
        // stack frame of this function.
        //
        // All instrumentation code must have no observable side effects.
        // In particular, all values of locals must be preserved and the
        // state of the operand stack must be preserved.
        let addr = self.instr.info.locations[bkpt_idx].address;
        let locs = self.debug_func().wasm_locations_at(addr);

        self.emit_operands(&locs, bkpt_idx)?;
        self.emit_locals(&locs, bkpt_idx)?;
        self.emit_globals(&locs, bkpt_idx)?;

        self.instructions
            .push(Instruction::I32Const(bkpt_idx as i32));
        self.instructions
            .push(Instruction::Call(self.instr.bkpt_fn_index));
        Ok(())
    }

    fn emit_operands(&mut self, locs: &WasmLocations, bkpt: usize) -> InstrResult {
        let Some(&first) = locs.operands.first() else {
            return Ok(());
        };
        let Some(&last) = locs.operands.last() else {
            return Ok(());
        };

        let height = self.validator.operand_stack_height() as usize;

        // If the highest operand index we need exceeds the number of operands we have
        // available, then it will be impossible to recover an operand value
        if last >= height {
            return error!(
                "Couldn't instrument operand {:?} with stack height {:?}",
                last, height
            );
        }

        let nlocals = self.validator.len_locals();

        for operand_idx in (first..height).rev() {
            let Some(Some(ty)) = self.validator.get_operand_type(height - operand_idx - 1) else {
                return error!(
                    "Couldn't instrument operand {:?}, unknown operand type",
                    operand_idx
                );
            };

            if !locs.operands.contains(&operand_idx) {}
        }

        Ok(())
    }

    fn emit_locals(&mut self, locs: &WasmLocations, bkpt: usize) -> InstrResult {
        for &local_idx in &locs.locals {
            let Some(ty) = self.validator.get_local_type(local_idx as u32) else {
                return error!("Couldn't get type of local {:?}", local_idx);
            };

            let Some(offset) = self
                .debug_func()
                .frame
                .place(WasmOp::Local(local_idx), ty, bkpt)
            else {
                continue;
            };

            self.instructions
                .push(Instruction::GlobalGet(self.instr.sp_gl_index));
            self.instructions
                .push(Instruction::LocalGet(local_idx as u32));

            self.emit_store(ty, offset);
        }

        Ok(())
    }

    fn emit_globals(&mut self, locs: &WasmLocations, bkpt: usize) -> InstrResult {
        let Some(types) = self.instr.validator.types(0) else {
            return error!("Could not get module types");
        };

        let globals: Vec<(usize, wasmparser::ValType)> = locs
            .globals
            .iter()
            .map(|&idx| (idx, types.global_at(idx as u32).content_type))
            .collect();

        for (global_idx, ty) in globals {
            let Some(offset) = self
                .debug_func()
                .frame
                .place(WasmOp::Global(global_idx), ty, bkpt)
            else {
                continue;
            };

            self.instructions
                .push(Instruction::GlobalGet(self.instr.sp_gl_index));
            self.instructions
                .push(Instruction::GlobalGet(global_idx as u32));
            self.emit_store(ty, offset);
        }

        Ok(())
    }

    fn emit_store(&mut self, ty: ValType, offset: usize) {
        let mem = MemArg {
            offset: offset as u64,
            align: 2,
            memory_index: self.instr.stack_mem_index,
        };
        self.instructions.push(match ty {
            ValType::I32 => Instruction::I32Store(mem),
            ValType::F32 => Instruction::F32Store(mem),
            ValType::I64 => Instruction::I64Store(mem),
            ValType::F64 => Instruction::F64Store(mem),
            ValType::V128 => Instruction::V128Store(mem),
            ValType::Ref(_) => unreachable!(),
        });
    }

    fn emit_footer(&mut self) {
        let instr_count = self.instructions.len();
        let frame_size = self.debug_func().frame.size;
        self.instructions.extend([
            Instruction::GlobalGet(self.instr.sp_gl_index),
            Instruction::I32Const(frame_size as i32),
            Instruction::I32Add,
            Instruction::GlobalSet(self.instr.sp_gl_index),
        ]);
        self.stack_intructions.push(instr_count + 1);
    }

    fn instrument(mut self) -> InstrResult<wasm_encoder::Function> {
        // Clear the stack frame for this function
        // This is a safety check to ensure we always start instrumentation at a known state.
        self.debug_func().frame.reset();
        self.emit_header();

        let mut reader = self
            .func_body
            .get_operators_reader()
            .map_err(reencode::Error::from)?;

        let body_rel_start = self.instr.code_ofs(self.func_body.range().start);
        let first_instr_rel = self.instr.code_ofs(reader.original_position());

        for code_ofs in body_rel_start..first_instr_rel {
            let Some(bkpt_idx) = self.instr.breakpoints.get(&code_ofs).copied() else {
                continue;
            };
            self.emit_bkpt(bkpt_idx)?;
        }

        while !reader.eof() {
            let (op, binary_ofs) = reader.read_with_offset().map_err(reencode::Error::from)?;
            let code_ofs = self.instr.code_ofs(binary_ofs);

            if let Some(&bkpt_idx) = self.instr.breakpoints.get(&code_ofs) {
                self.emit_bkpt(bkpt_idx)?;
            }

            // Pass this operator to the wasmparser validator. It will internally
            // update its state to keep track of operand stack types, depth, etc.
            // according to the instruction given.
            //
            // It is important that this is run *after* doing instrumentation code for
            // this instruction's breakpoint, if any, because breakpoints should stop the
            // code immediately before the instruction has run. In other words, the
            // instrumentation code should not have observed the effects of that instruction yet.
            self.validator.op(binary_ofs, &op)?;

            match op {
                wasmparser::Operator::Return => {
                    self.emit_footer();
                    self.instructions.push(Instruction::Return);
                }
                wasmparser::Operator::ReturnCall { function_index } => {
                    self.instructions.push(
                        self.instr
                            .instruction(wasmparser::Operator::Call { function_index })?,
                    );
                    self.emit_footer();
                    self.instructions.push(Instruction::Return);
                }
                wasmparser::Operator::ReturnCallIndirect {
                    type_index,
                    table_index,
                } => {
                    self.instructions.push(self.instr.instruction(
                        wasmparser::Operator::CallIndirect {
                            type_index,
                            table_index,
                        },
                    )?);
                    self.emit_footer();
                    self.instructions.push(Instruction::Return);
                }
                wasmparser::Operator::ReturnCallRef { type_index } => {
                    self.instructions.push(
                        self.instr
                            .instruction(wasmparser::Operator::CallRef { type_index })?,
                    );
                    self.emit_footer();
                    self.instructions.push(Instruction::Return);
                }
                wasmparser::Operator::End => {
                    if reader.eof() {
                        self.emit_footer();
                    }
                    self.instructions.push(self.instr.instruction(op)?);
                }
                _ => {
                    self.instructions.push(self.instr.instruction(op)?);
                }
            }
        }

        reader.finish()?;

        /* Adjust stack instructions to include stack size */
        let frame_size = self.debug_func().frame.size;
        for instr_index in self.stack_intructions {
            let inst = &mut self.instructions[instr_index];
            assert!(matches!(*inst, Instruction::I32Const(_)));
            *inst = Instruction::I32Const(frame_size as i32);
        }

        /* Compute locals of emitted function (old locals + scratch locals) */
        let mut locals: Vec<(u32, wasm_encoder::ValType)> = Vec::new();
        for pair in self
            .func_body
            .get_locals_reader()
            .map_err(reencode::Error::from)?
        {
            let (cnt, ty) = pair.map_err(reencode::Error::from)?;
            locals.push((cnt, reencode::Reencode::val_type(self.instr, ty)?));
        }
        for ty in &self.scratch_locals {
            locals.push((1, *ty));
        }

        /* Emit the new function with new instructions and return */
        let mut func = wasm_encoder::Function::new(locals);
        for inst in &self.instructions {
            func.instruction(inst);
        }

        Ok(func)
    }
}

/// Instrument a WASM binary to support debugging
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
