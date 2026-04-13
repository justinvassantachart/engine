use super::{InstrResult, Instrumenter};
use crate::debug::dwarf::{Die, get_location, get_variables};
use crate::debug::instrument::InstrError;
use crate::types::{DebugFrameEntry, DebugFunction, GlobalAddress, WasmLocation};
use std::collections::{BTreeSet, HashMap, HashSet};
use wasm_encoder::reencode::Reencode;
use wasm_encoder::{Instruction, MemArg, reencode};
use wasmparser::ValType;

macro_rules! error {
    ($($arg:tt)*) => {
        Err($crate::debug::instrument::InstrError::UserError(
            $crate::debug::instrument::Error::msg(format!($($arg)*)),
        ))
    };
}

#[derive(Default)]
struct WasmLocations {
    operands: BTreeSet<usize>,
    locals: BTreeSet<usize>,
    globals: BTreeSet<usize>,
}

pub struct FnInstrumenter<'a, 'b, 'c> {
    instr: &'a mut Instrumenter<'b>,
    func_idx: usize,
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
    scratch_locals: Vec<wasmparser::ValType>,
}

impl<'a, 'b, 'c> FnInstrumenter<'a, 'b, 'c> {
    pub fn new(
        instr: &'a mut Instrumenter<'b>,
        func_idx: usize,
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
            func_idx,
            func_body,
            validator,

            instructions: Vec::default(),
            stack_intructions: Vec::default(),
            scratch_locals: Vec::default(),
        })
    }

    fn func(&self) -> &DebugFunction {
        &self.instr.info.functions[self.func_idx]
    }

    fn func_mut(&mut self) -> &mut DebugFunction {
        &mut self.instr.info.functions[self.func_idx]
    }

    fn emit_header(&mut self) {
        let instr_count = self.instructions.len();
        let frame_size = self.func_mut().size;
        self.instructions.extend([
            Instruction::GlobalGet(self.instr.sp_gl_index),
            Instruction::I32Const(frame_size as i32),
            Instruction::I32Sub,
            Instruction::GlobalSet(self.instr.sp_gl_index),
            Instruction::GlobalGet(self.instr.sp_gl_index),
            Instruction::I32Const(self.func_idx as i32),
            Instruction::I32Store(MemArg {
                offset: 0,
                align: 2,
                memory_index: self.instr.stack_mem_index,
            }),
        ]);
        self.stack_intructions.push(instr_count + 1);
    }

    fn locations_at(&self, pc: GlobalAddress) -> InstrResult<WasmLocations> {
        let fun = self
            .func()
            .die_ref
            .deref(&self.instr.info.dwarf)
            .map_err(|e| InstrError::UserError(e))?;

        let vars = get_variables(&fun, pc);
        let mut locs = WasmLocations::default();

        for var in vars {
            let Some(expr) = get_location(&var, pc) else {
                continue;
            };

            for op in expr.operations(fun.ctx().unit.unit().encoding()) {
                let op = op.map_err(|e| InstrError::UserError(e.into()))?;
                match op {
                    gimli::Operation::WasmLocal { index } => locs.locals.insert(index as usize),
                    gimli::Operation::WasmGlobal { index } => locs.globals.insert(index as usize),
                    gimli::Operation::WasmStack { index } => locs.operands.insert(index as usize),
                    _ => true,
                };
            }
        }

        Ok(WasmLocations::default())
    }

    fn emit_bkpt(&mut self, bkpt_idx: usize, pc: GlobalAddress) -> InstrResult {
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
        let locs = self.locations_at(pc)?;

        self.emit_operands(&locs)?;
        self.emit_locals(&locs)?;
        self.emit_globals(&locs)?;

        self.instructions
            .push(Instruction::I32Const(bkpt_idx as i32));
        self.instructions
            .push(Instruction::Call(self.instr.bkpt_fn_index));
        Ok(())
    }

    /// Emits instrumentation code to recover the operands in [WasmLocations::operands].
    ///
    /// The basic strategy is:
    ///     - Pop all values off of the operand stack, storing each in a scratch local variable
    ///     - If the popped value is one of the values we care about, then store it into the debug stack.
    ///     - Push all values stoerd in scratch locals back onto the operand stack.
    ///
    /// Note that this approach will be underperformant for large stack sizes.
    /// In the future, an optimization we can perform would be to recover an operand
    /// preemptively as soon as it is pushed by recognizing in advance that its value will
    /// be needed at a later point in the program. This would avoid the need to unroll the
    /// stack in this manner.
    fn emit_operands(&mut self, locs: &WasmLocations) -> InstrResult {
        let Some(&first) = locs.operands.first() else {
            return Ok(());
        };
        let Some(&last) = locs.operands.last() else {
            return Ok(());
        };

        let height = self.validator.operand_stack_height() as usize;

        // If the indices of the operands that we need exceed the number of operands we have
        // available, then it will be impossible to recover an operand value
        if first >= height || last >= height {
            return error!(
                "Couldn't instrument operands {:?}-{:?} with stack height {:?}",
                first, last, height
            );
        }

        let nlocals = self.validator.len_locals() as usize;

        // `scratch_indices` contains the indices in `self.scratch_locals`
        // which have been consumed to store operands while unrolling the operand stack.
        //
        // `scratch_map` maps operand stack indices to their corresponding index in `self.scratch_locals`
        // to be used when returning values to the operand stack.
        let mut scratch_indices = HashSet::new();
        let mut scratch_map = HashMap::new();

        // Loop through operands, starting from the top of the operand stack
        // and ending with the bottom-most operand that we care about
        for operand_idx in (first..height).rev() {
            // Get the type of this operand using
            let Some(Some(ty)) = self.validator.get_operand_type(height - operand_idx - 1) else {
                return error!(
                    "Couldn't instrument operand {:?}, unknown operand type",
                    operand_idx
                );
            };

            // Get or allocate a scratch local to store this stack operand
            let scratch_idx = self
                .scratch_locals
                .iter()
                .enumerate()
                .position(|(scratch_idx, &scratch_ty)| {
                    !scratch_indices.contains(&scratch_idx) && scratch_ty == ty
                })
                .unwrap_or_else(|| {
                    let scratch_idx = self.scratch_locals.len();
                    self.scratch_locals.push(ty);
                    scratch_indices.insert(scratch_idx);
                    scratch_map.insert(operand_idx, scratch_idx);
                    scratch_idx
                });

            let scratch_idx = (scratch_idx + nlocals) as u32;

            // Consume the operand at index `operand_idx` by pushing it to the scratch local
            self.instructions.push(Instruction::LocalSet(scratch_idx));

            // Store the operand value to the debug stack
            if locs.operands.contains(&operand_idx) {
                let offset = self.func_mut().place(WasmLocation::Operand(operand_idx));
                self.instructions
                    .push(Instruction::GlobalGet(self.instr.sp_gl_index));
                self.instructions.push(Instruction::LocalGet(scratch_idx));
                self.emit_store(ty, offset);
            }
        }

        // Loop through operands, starting with the bottom-most operand that we care about
        // and ending with the original top value in the stack
        for operand_idx in first..height {
            let Some(&scratch_idx) = scratch_map.get(&operand_idx) else {
                return error!(
                    "Could not recover operand {:?}: no corresponding scratch local",
                    operand_idx
                );
            };

            let scratch_idx = (scratch_idx + nlocals) as u32;
            self.instructions.push(Instruction::LocalGet(scratch_idx));
        }

        Ok(())
    }

    fn emit_locals(&mut self, locs: &WasmLocations) -> InstrResult {
        for &local_idx in &locs.locals {
            let Some(ty) = self.validator.get_local_type(local_idx as u32) else {
                return error!("Couldn't get type of local {:?}", local_idx);
            };

            let offset = self.func_mut().place(WasmLocation::Local(local_idx));
            self.instructions
                .push(Instruction::GlobalGet(self.instr.sp_gl_index));
            self.instructions
                .push(Instruction::LocalGet(local_idx as u32));

            self.emit_store(ty, offset);
        }

        Ok(())
    }

    fn emit_globals(&mut self, locs: &WasmLocations) -> InstrResult {
        let Some(types) = self.instr.validator.types(0) else {
            return error!("Could not get module types");
        };

        let globals: Vec<(usize, wasmparser::ValType)> = locs
            .globals
            .iter()
            .map(|&idx| (idx, types.global_at(idx as u32).content_type))
            .collect();

        for (global_idx, ty) in globals {
            let offset = self.func_mut().place(WasmLocation::Global(global_idx));

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
        let frame_size = self.func_mut().size;
        self.instructions.extend([
            Instruction::GlobalGet(self.instr.sp_gl_index),
            Instruction::I32Const(frame_size as i32),
            Instruction::I32Add,
            Instruction::GlobalSet(self.instr.sp_gl_index),
        ]);
        self.stack_intructions.push(instr_count + 1);
    }

    pub fn instrument(mut self) -> InstrResult<wasm_encoder::Function> {
        // Clear the stack frame for this function
        // This is a safety check to ensure we always start instrumentation at a known state.
        self.func_mut().reset();
        self.emit_header();

        let mut reader = self
            .func_body
            .get_operators_reader()
            .map_err(reencode::Error::from)?;

        let body_rel_start = self.instr.code_ofs(self.func_body.range().start);
        let first_instr_rel = self.instr.code_ofs(reader.original_position());

        for code_ofs in body_rel_start.0..first_instr_rel.0 {
            let code_ofs = GlobalAddress(code_ofs);
            let Some(bkpt_idx) = self.instr.breakpoints.get(&code_ofs).copied() else {
                continue;
            };
            self.emit_bkpt(bkpt_idx, code_ofs)?;
        }

        while !reader.eof() {
            let (op, binary_ofs) = reader.read_with_offset().map_err(reencode::Error::from)?;
            let code_ofs = self.instr.code_ofs(binary_ofs);

            if let Some(&bkpt_idx) = self.instr.breakpoints.get(&code_ofs) {
                self.emit_bkpt(bkpt_idx, code_ofs)?;
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
        let frame_size = self.func_mut().size;
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
            locals.push((1, reencode::Reencode::val_type(self.instr, *ty)?));
        }

        /* Emit the new function with new instructions and return */
        let mut func = wasm_encoder::Function::new(locals);
        for inst in &self.instructions {
            func.instruction(inst);
        }

        Ok(func)
    }
}
