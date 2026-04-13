use crate::types::{DebugInfo, GlobalAddress};
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub r#type: Option<String>,
}

// ---------------------------------------------------------------------------
// Main-thread Debugger
// ---------------------------------------------------------------------------

/// Main-thread debugger that operates on shared memory from an attached worker.
/// Constructed from `DebugInfo` received via the worker's `debug` message.
pub struct Debugger {
    info: DebugInfo,
    state: js_sys::Int32Array,
}

impl Debugger {
    pub fn new(info: DebugInfo) -> Self {
        let state = info.get_bp_state();
        Self { info, state }
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
    pub fn set_breakpoints(&self, file: &str, lines: &[i64]) -> Vec<(i64, bool)> {
        Vec::new() // TODO
    }

    pub fn get_variables(&self, _frame_id: u32) -> Vec<Variable> {
        Vec::new() // TODO: resolve variables from debug stack + DWARF
    }

    /// Resumes the worker by signaling through the SAB.
    pub fn continue_(&self) {
        js_sys::Atomics::store(&self.state, 0, 0).unwrap();
        js_sys::Atomics::notify(&self.state, 0).unwrap();
    }
}
