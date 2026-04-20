use std::rc::Rc;

use crate::debug::dwarf::{
    Type, TypeGraph, Value, get_location, get_variables as dwarf_get_variables,
};
use crate::types::{DebugFunction, DebugInfo, GlobalAddress, WasmLocation};
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

pub struct Variable {
    pub name: String,
    pub value: Value,
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

            frames.push(StackFrame {
                id: frames.len() as u32,
                name: die.name().unwrap_or(String::new()),
                line: 0,
                column: 0,
                source: None,
            });
            pos += func.size as u32;
        }

        Ok(frames)
    }

    /// Replaces breakpoints for the given source file.
    /// Returns a list of `(line, verified)` pairs.
    pub fn set_breakpoints(&self, _file: &str, _lines: &[i64]) -> Vec<(i64, bool)> {
        Vec::new() // TODO
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

    pub fn get_variables(&self, frame_id: u32) -> Vec<Variable> {
        let Some((pos, pc, func)) = self.frame_at(frame_id) else {
            return Vec::new();
        };
        let Ok(die) = func.die_ref.deref(&self.info.dwarf) else {
            return Vec::new();
        };

        let (view, _, _) = self.stack_view();
        let var_dies = dwarf_get_variables(&die, pc);
        let encoding: gimli::Encoding = die.ctx().unit.unit().encoding();
        let mut variables = Vec::new();

        for var_die in &var_dies {
            let name = var_die.name().unwrap_or_default();
            let Some(expr) = get_location(var_die, pc) else {
                continue;
            };

            let mut eval = expr.evaluation(encoding);
            let pieces = loop {
                match eval.evaluate() {
                    Ok(gimli::EvaluationResult::Complete) => break eval.result(),
                    Ok(gimli::EvaluationResult::RequiresWasmLocal { index }) => {
                        let loc = WasmLocation::Local(index as usize);
                        let Some(val) = self.read_wasm_value(&view, pos, func, &loc) else {
                            break vec![];
                        };
                        let _ = eval.resume_with_wasm_value(val);
                    }
                    Ok(gimli::EvaluationResult::RequiresWasmGlobal { index }) => {
                        let loc = WasmLocation::Global(index as usize);
                        let Some(val) = self.read_wasm_value(&view, pos, func, &loc) else {
                            break vec![];
                        };
                        let _ = eval.resume_with_wasm_value(val);
                    }
                    Ok(gimli::EvaluationResult::RequiresWasmStack { index }) => {
                        let loc = WasmLocation::Operand(index as usize);
                        let Some(val) = self.read_wasm_value(&view, pos, func, &loc) else {
                            break vec![];
                        };
                        let _ = eval.resume_with_wasm_value(val);
                    }
                    _ => break vec![],
                }
            };
            if pieces.is_empty() {
                continue;
            }
            let Some(type_id) = var_die.type_ref() else {
                continue;
            };
            variables.push(Variable {
                name,
                value: Value::new(pieces, Type::new(type_id, self.types.clone())),
            });
        }
        variables
    }

    /// Resumes the worker by signaling through the SAB.
    pub fn continue_(&self) {
        js_sys::Atomics::store(&self.state, 0, 0).unwrap();
        js_sys::Atomics::notify(&self.state, 0).unwrap();
    }
}
