use crate::dap::types::{StackFrame, Variable};
use crate::types::DebugInfo;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Adapter {
    info: DebugInfo,
}

#[wasm_bindgen]
impl Adapter {
    #[wasm_bindgen(constructor)]
    pub fn new(info: JsValue) -> Result<Adapter, JsError> {
        let info: DebugInfo =
            serde_wasm_bindgen::from_value(info).map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Adapter { info })
    }

    fn sentinel(&self) -> js_sys::Int32Array {
        js_sys::Int32Array::new_with_byte_offset_and_length(&self.info.breakpoints, 0, 4)
    }

    /// Replaces all breakpoints for a given file.
    /// Pass an empty slice to clear all breakpoints in that file.
    #[wasm_bindgen(js_name = "setBreakpoints")]
    pub fn set_breakpoints(&mut self, file: &str, lines: &[u32]) {
        unimplemented!("set_breakpoints is not implemented");
    }

    #[wasm_bindgen(js_name = "stackTrace")]
    pub fn stack_trace(&self) -> Result<JsValue, JsError> {
        let sentinel = self.sentinel();
        let sp = sentinel.get_index(3) as u32;

        let mut frames: Vec<StackFrame> = Vec::new();
        let buffer = self.info.stack.memory.buffer();
        let buffer = buffer.unchecked_ref::<js_sys::ArrayBuffer>();
        let stack_top = buffer.byte_length();
        let stack_view = js_sys::DataView::new(buffer, 0, stack_top as usize);
        let mut pos = sp;

        // Walk the stack
        while pos < stack_top {
            let func_idx = stack_view.get_uint32_endian(pos as usize, true) as usize;

            // Edge case, should never be true
            if func_idx >= self.info.functions.len() {
                break;
            }

            let func = &self.info.functions[func_idx];
            frames.push(StackFrame {
                id: frames.len() as u32,
                name: func.name.clone(),
                // TODO: should figure this out
                line: 0,
                column: 0,
            });
            pos += func.size as u32;
        }

        serde_wasm_bindgen::to_value(&frames).map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen(js_name = "getVariables")]
    pub fn get_variables(&self) -> Result<JsValue, JsError> {
        let vars: Vec<Variable> = Vec::new(); // TODO: resolve variables
        serde_wasm_bindgen::to_value(&vars).map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen(js_name = "continue")]
    pub fn continue_(&self) {
        // Write to the sentinel value
        let sentinel =
            js_sys::Int32Array::new_with_byte_offset_and_length(&self.info.breakpoints, 0, 1);

        js_sys::Atomics::add(&sentinel, 0, 1).unwrap();
        js_sys::Atomics::notify(&sentinel, 0).unwrap();
    }
}
