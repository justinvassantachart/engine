use crate::types::DebugInfo;
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

pub const BREAKPOINT_PREFIX_BYTES: usize = 16;

// ---------------------------------------------------------------------------
// Main-thread Debugger
// ---------------------------------------------------------------------------

/// Main-thread debugger that operates on shared memory from an attached worker.
/// Constructed from `DebugInfo` received via the worker's `debug` message.
pub struct Debugger {
    info: DebugInfo,
    sentinel: js_sys::Int32Array,
    flags: js_sys::Uint8Array,
}

impl Debugger {
    pub fn new(info: DebugInfo) -> Self {
        let sentinel = js_sys::Int32Array::new_with_byte_offset_and_length(&info.breakpoints, 0, 4);
        let flags = js_sys::Uint8Array::new_with_byte_offset(
            &info.breakpoints,
            BREAKPOINT_PREFIX_BYTES as u32,
        );
        Self {
            info,
            sentinel,
            flags,
        }
    }

    /// Walks the debug stack and returns the current call stack.
    pub fn backtrace(&self) -> anyhow::Result<Vec<StackFrame>> {
        let sp = self.sentinel.get_index(3) as u32;
        let buffer = self.info.stack.memory.buffer();
        let buffer = buffer.unchecked_ref::<js_sys::ArrayBuffer>();
        let stack_top = buffer.byte_length();
        let stack_view = js_sys::DataView::new(buffer, 0, stack_top as usize);

        let mut frames = Vec::new();
        let mut pos = sp;

        while pos < stack_top {
            let func_idx = stack_view.get_uint32_endian(pos as usize, true) as usize;
            if func_idx >= self.info.functions.len() {
                break;
            }

            let func = &self.info.functions[func_idx];
            let die = func.die_ref.deref(&self.info.dwarf)?;

            frames.push(StackFrame {
                id: frames.len() as u32,
                name: die.name().unwrap_or(String::new()),
                line: 0, // TODO: resolve from DWARF
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

    /// Resumes the worker by signaling through the sentinel.
    pub fn continue_(&self) {
        js_sys::Atomics::add(&self.sentinel, 0, 1).unwrap();
        js_sys::Atomics::notify(&self.sentinel, 0).unwrap();
    }
}
