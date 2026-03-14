//! Debug host API exposed to the main thread.
//!
//! Runs in the main-thread (browser) WASM runtime. Receives DebugInfo from the worker
//! and provides get_frames() / get_variables() to unwrap the debug stack when paused.
//! The worker saves the stack pointer into the breakpoints buffer before sending a
//! Breakpoint message so the host can read it (the host has no access to the SP global).

use crate::debug::BREAKPOINT_PREFIX_BYTES;
use crate::types::DebugInfo;
use js_sys::Reflect;
use wasm_bindgen::prelude::*;

/// A single frame in the call stack when execution is paused.
#[wasm_bindgen]
pub struct StackFrame {
    name: String,
    /// Index into DebugInfo.functions (order from DWARF parsing, not source order).
    function: u32,
}

#[wasm_bindgen]
impl StackFrame {
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn function(&self) -> u32 {
        self.function
    }

    /// Variable eval will happen in Rust (new model). Returns empty until implemented.
    #[wasm_bindgen(getter)]
    pub fn variables(&self) -> js_sys::Array {
        js_sys::Array::new()
    }
}

#[wasm_bindgen]
pub struct Variable {
    name: String,
    ty: String,
    value: String,
    variables_ref: Option<usize>,
}

#[wasm_bindgen]
pub struct DebugHost {
    info: DebugInfo,
}

#[wasm_bindgen]
impl DebugHost {
    #[wasm_bindgen(constructor)]
    pub fn new(info: JsValue) -> Result<DebugHost, JsValue> {
        let info: DebugInfo =
            serde_wasm_bindgen::from_value(info).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(DebugHost { info })
    }

    #[wasm_bindgen]
    pub fn get_frames(&mut self) -> Vec<StackFrame> {
        // Breakpoints buffer layout (first 16 bytes = 4 u32s):
        //   [0] Sentinel (pause/resume), [1] Mode, [2] Breakpoint index, [3] Saved SP
        let meta = js_sys::Uint32Array::new_with_byte_offset_and_length(
            &self.info.breakpoints,
            0,
            BREAKPOINT_PREFIX_BYTES as u32 / 4,
        );
        let sp = meta.get_index(3);
        if sp == 0 {
            return vec![]; // Not paused, or worker didn't write SP yet
        }

        // Stack memory is WebAssembly.Memory; get its underlying buffer (SharedArrayBuffer)
        // shared with the worker. The instrumented WASM writes frame data here.
        let stack_buf = Reflect::get(self.info.stack.memory.as_ref(), &"buffer".into())
            .unwrap_or(JsValue::NULL);
        let stack_len: u32 = Reflect::get(&stack_buf, &"byteLength".into())
            .ok()
            .and_then(|v| v.as_f64())
            .map(|f| f as u32)
            .unwrap_or(0);
        if stack_len == 0 {
            return vec![];
        }

        let mut frames = Vec::new();
        let mut offset: u32 = sp;

        // Each frame layout: [func_idx: u32][layout bytes...]. Stack grows down;
        // caller frames are at higher addresses, so we advance offset += size.
        while offset + 4 <= stack_len {
            let u32_view =
                js_sys::Uint32Array::new_with_byte_offset_and_length(&stack_buf, offset, 1);
            let func_idx = u32_view.get_index(0);
            let Some(debug_fn) = self.info.functions.get(func_idx as usize) else {
                break;
            };

            let size = debug_fn.size as u32;
            frames.push(StackFrame {
                name: format!("function_{}", func_idx),
                function: func_idx,
            });

            offset += size;
            if size == 0 {
                break;
            }
        }

        frames
    }

    pub fn get_variables(&mut self, _variables_ref: usize) -> Vec<StackFrame> {
        vec![]
    }
}
