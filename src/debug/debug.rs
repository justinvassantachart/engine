use std::rc::Rc;

use crate::debug::{Type, TypeGraph, Variable, get_location, get_variables as debug_get_variables};
use crate::types::{
    BKPT_MODE_NORMAL, BKPT_MODE_STEP_INTO, BKPT_MODE_STEP_OUT, BKPT_MODE_STEP_OVER,
    DebugFunction, DebugInfo, GlobalAddress, WasmLocation,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StackFrame {
    pub id: u32,
    pub name: String,
    pub line: u32,
    pub column: u32,
    pub source: Option<String>,
}

// ---------------------------------------------------------------------------
// Main-thread Debugger
// ---------------------------------------------------------------------------

/// Main-thread debugger that operates on shared memory from an attached worker.
/// Constructed from `DebugInfo` received via the worker's `debug` message.
pub struct Debugger {
    info: DebugInfo,
    types: Rc<TypeGraph>,
    state: js_sys::Int32Array,
}

impl Debugger {
    pub fn new(info: DebugInfo) -> Self {
        let state = info.get_bp_state();
        let types = Rc::from(TypeGraph::new(&info.dwarf));
        Self { info, state, types }
    }

    /// Breakpoint mode that was active when the worker last chose to pause (SAB index 3).
    pub fn last_pause_mode(&self) -> i32 {
        js_sys::Atomics::load(&self.state, 3).unwrap_or(BKPT_MODE_NORMAL)
    }

    fn read_wasm_value(
        &self,
        view: &js_sys::DataView,
        frame_pos: u32,
        func: &DebugFunction,
        loc: &WasmLocation,
    ) -> Option<gimli::Value> {
        let entry = func.layout.iter().find(|e| &e.location == loc)?;
        let offset = (frame_pos as usize) + entry.offset;
        let raw = view.get_uint32_endian(offset, true) as u64;
        Some(gimli::Value::Generic(raw))
    }

    fn stack_view(&self) -> (js_sys::DataView, u32, u32) {
        let sp = self.state.get_index(0) as u32;
        let buffer = self.info.stack.memory.buffer();
        let buffer = buffer.unchecked_ref::<js_sys::ArrayBuffer>();
        let stack_top = buffer.byte_length();
        let view = js_sys::DataView::new(buffer, 0, stack_top as usize);
        (view, sp, stack_top)
    }

    /// Walks the debug stack and returns the current call stack.
    pub fn backtrace(&self) -> anyhow::Result<Vec<StackFrame>> {
        let (view, sp, stack_top) = self.stack_view();
        let mut frames = Vec::new();
        let mut pos = sp;

        while pos < stack_top {
            let pc = GlobalAddress(view.get_uint32_endian(pos as usize, true) as u64);
            let func = match self.info.fn_at(pc) {
                Some(f) => f,
                None => break,
            };
            let die = func.die_ref.deref(&self.info.dwarf)?;

            let loc = self
                .info
                .dwarf
                .locations()
                .filter(|l| l.address() <= pc)
                .max_by_key(|l| l.address());
            frames.push(StackFrame {
                id: frames.len() as u32,
                name: die.name().unwrap_or(String::new()),
                line: loc.as_ref().map_or(0, |l| l.line() as u32),
                column: loc.as_ref().map_or(0, |l| l.column() as u32),
                source: loc.as_ref().map(|l| l.file.to_string_lossy().into_owned()),
            });
            pos += func.size as u32;
        }

        Ok(frames)
    }

    /// Sets one breakpoint for one `(file, line)` pair.
    pub fn set_breakpoint(&self, file: &str, line: i64) -> bool {
        let flags = self.info.get_bp_flags();

        let target = std::path::Path::new(file);
        let target = if target.is_absolute() {
            target.to_path_buf()
        } else {
            std::path::Path::new("/").join(target)
        };
        let mut verified = false;

        for (idx, loc) in self.info.dwarf.locations().enumerate() {
            if loc.file != target {
                continue;
            }
            let loc_line = loc.line() as i64;
            if loc_line == line {
                flags.set_index(idx as u32, 1);
                verified = true;
            }
        }

        verified
    }

    /// Walks the debug stack to the Nth frame and returns (position, pc, func).
    fn frame_at(
        &self,
        frame_id: u32,
    ) -> Option<(u32, GlobalAddress, &crate::types::DebugFunction)> {
        let (view, sp, stack_top) = self.stack_view();
        let mut pos = sp;

        for _ in 0..frame_id {
            if pos >= stack_top {
                return None;
            }
            let pc = GlobalAddress(view.get_uint32_endian(pos as usize, true) as u64);
            let func = self.info.fn_at(pc)?;
            pos += func.size as u32;
        }

        if pos >= stack_top {
            return None;
        }
        let pc = GlobalAddress(view.get_uint32_endian(pos as usize, true) as u64);
        let func = self.info.fn_at(pc)?;
        Some((pos, pc, func))
    }

    /// Evaluates a DWARF expression, satisfying any wasm-location and
    /// frame-base requests by reading the function's debug stack frame.
    ///
    /// Returns an empty vector if the expression yields a request kind we
    /// don't know how to fulfil (e.g. `RequiresMemory`).
    fn evaluate_expr(
        &self,
        expr: gimli::Expression<crate::debug::dwarf::R>,
        encoding: gimli::Encoding,
        view: &js_sys::DataView,
        pos: u32,
        func: &DebugFunction,
        frame_base: Option<u64>,
    ) -> Vec<gimli::Piece<crate::debug::dwarf::R>> {
        let mut eval = expr.evaluation(encoding);
        loop {
            match eval.evaluate() {
                Ok(gimli::EvaluationResult::Complete) => return eval.result(),
                Ok(gimli::EvaluationResult::RequiresWasmLocal { index }) => {
                    let Some(val) =
                        self.read_wasm_value(view, pos, func, &WasmLocation::Local(index as usize))
                    else {
                        return vec![];
                    };
                    if eval.resume_with_wasm_value(val).is_err() {
                        return vec![];
                    }
                }
                Ok(gimli::EvaluationResult::RequiresWasmGlobal { index }) => {
                    let Some(val) = self.read_wasm_value(
                        view,
                        pos,
                        func,
                        &WasmLocation::Global(index as usize),
                    ) else {
                        return vec![];
                    };
                    if eval.resume_with_wasm_value(val).is_err() {
                        return vec![];
                    }
                }
                Ok(gimli::EvaluationResult::RequiresWasmStack { index }) => {
                    let Some(val) = self.read_wasm_value(
                        view,
                        pos,
                        func,
                        &WasmLocation::Operand(index as usize),
                    ) else {
                        return vec![];
                    };
                    if eval.resume_with_wasm_value(val).is_err() {
                        return vec![];
                    }
                }
                Ok(gimli::EvaluationResult::RequiresFrameBase) => {
                    let Some(fb) = frame_base else {
                        return vec![];
                    };
                    if eval.resume_with_frame_base(fb).is_err() {
                        return vec![];
                    }
                }
                _ => return vec![],
            }
        }
    }

    /// Evaluates the function's `DW_AT_frame_base` expression and reduces the
    /// resulting pieces to a single u64 the variable evaluator can use.
    fn frame_base(
        &self,
        die: &crate::debug::dwarf::Die<'_>,
        pc: GlobalAddress,
        encoding: gimli::Encoding,
        view: &js_sys::DataView,
        pos: u32,
        func: &DebugFunction,
    ) -> Option<u64> {
        let expr = die.expression(gimli::DW_AT_frame_base, pc)?;
        let pieces = self.evaluate_expr(expr, encoding, view, pos, func, None);
        let piece = pieces.first()?;
        match &piece.location {
            gimli::Location::Value { value } => Some(gimli_value_to_u64(*value)),
            gimli::Location::Address { address } => Some(*address),
            _ => None,
        }
    }

    /// Returns the variables visible in `frame_id`, split into `(arguments, locals)`.
    ///
    /// Arguments are DIE children tagged `DW_TAG_formal_parameter`; locals are
    /// `DW_TAG_variable` (the modern tag) or `DW_TAG_local_variable`. Variables
    /// whose location expression cannot be resolved (e.g. optimized out /
    /// require unsupported opcodes) are dropped.
    pub fn get_variables(&self, frame_id: u32) -> (Vec<Variable>, Vec<Variable>) {
        let Some((pos, pc, func)) = self.frame_at(frame_id) else {
            return (Vec::new(), Vec::new());
        };
        let Ok(die) = func.die_ref.deref(&self.info.dwarf) else {
            return (Vec::new(), Vec::new());
        };

        let (view, _, _) = self.stack_view();
        let var_dies = debug_get_variables(&die, pc);
        let encoding: gimli::Encoding = die.ctx().unit.unit().encoding();
        let frame_base = self.frame_base(&die, pc, encoding, &view, pos, func);
        let mut arguments = Vec::new();
        let mut locals = Vec::new();

        for var_die in &var_dies {
            let name = var_die.name().unwrap_or_default();
            let Some(expr) = get_location(var_die, pc) else {
                continue;
            };

            let pieces = self.evaluate_expr(expr, encoding, &view, pos, func, frame_base);
            if pieces.is_empty() {
                continue;
            }
            let Some(type_id) = var_die.type_ref() else {
                continue;
            };
            let variable = Variable::new(name, pieces, Type::new(type_id, self.types.clone()));
            match var_die.tag() {
                gimli::DW_TAG_formal_parameter => arguments.push(variable),
                gimli::DW_TAG_variable | gimli::DW_TAG_local_variable => locals.push(variable),
                _ => {}
            }
        }
        (arguments, locals)
    }

    /// Borrow of the underlying [`DebugInfo`], used by handlers that need to
    /// peek at memory or DWARF without owning the debugger.
    pub fn info(&self) -> &DebugInfo {
        &self.info
    }

    fn notify_resume(&self) {
        js_sys::Atomics::store(&self.state, 0, 0).unwrap();
        js_sys::Atomics::notify(&self.state, 0).unwrap();
    }

    /// Resumes the worker by signaling through the SAB (normal execution: breakpoints only).
    pub fn continue_(&self) {
        js_sys::Atomics::store(&self.state, 1, BKPT_MODE_NORMAL).unwrap();
        self.notify_resume();
    }

    /// Resume with step-into semantics (stop at the next instrumented breakpoint site).
    pub fn step_into(&self) {
        js_sys::Atomics::store(&self.state, 1, BKPT_MODE_STEP_INTO).unwrap();
        self.notify_resume();
    }

    /// Resume with step-over semantics (same or outer frame versus `last_sp`).
    pub fn step_over(&self) {
        js_sys::Atomics::store(&self.state, 1, BKPT_MODE_STEP_OVER).unwrap();
        self.notify_resume();
    }

    /// Resume with step-out semantics (outer frame only versus `last_sp`).
    pub fn step_out(&self) {
        js_sys::Atomics::store(&self.state, 1, BKPT_MODE_STEP_OUT).unwrap();
        self.notify_resume();
    }
}

fn gimli_value_to_u64(v: gimli::Value) -> u64 {
    match v {
        gimli::Value::Generic(x) => x,
        gimli::Value::I8(x) => x as i64 as u64,
        gimli::Value::U8(x) => x as u64,
        gimli::Value::I16(x) => x as i64 as u64,
        gimli::Value::U16(x) => x as u64,
        gimli::Value::I32(x) => x as i64 as u64,
        gimli::Value::U32(x) => x as u64,
        gimli::Value::I64(x) => x as u64,
        gimli::Value::U64(x) => x,
        gimli::Value::F32(_) | gimli::Value::F64(_) => 0,
    }
}
