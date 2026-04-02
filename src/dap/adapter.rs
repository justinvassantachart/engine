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

    /// Replaces all breakpoints for a given file.
    /// Pass an empty slice to clear all breakpoints in that file.
    #[wasm_bindgen(js_name = "setBreakpoints")]
    pub fn set_breakpoints(&mut self, file: &str, lines: &[u32]) {
        unimplemented!("set_breakpoints is not implemented");
    }

    #[wasm_bindgen(js_name = "stackTrace")]
    pub fn stack_trace(&self) -> Result<JsValue, JsError> {
        let frames: Vec<StackFrame> = Vec::new(); // TODO: walk debug stack
        serde_wasm_bindgen::to_value(&frames).map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen(js_name = "getVariables")]
    pub fn get_variables(&self) -> Result<JsValue, JsError> {
        let vars: Vec<Variable> = Vec::new(); // TODO: resolve variables
        serde_wasm_bindgen::to_value(&vars).map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen(js_name = "continue")]
    pub fn continue_(&self) {
        /// write to the sentinel value
        let sentinel =
            js_sys::Int32Array::new_with_byte_offset_and_length(&self.info.breakpoints, 0, 1);

        js_sys::Atomics::add(&sentinel, 0, 1).unwrap();
        js_sys::Atomics::notify(&sentinel, 0).unwrap();
    }
}
