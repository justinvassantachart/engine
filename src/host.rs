use crate::{log, types::DebugInfo};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct StackFrame {
    name: String,
}

#[wasm_bindgen]
impl StackFrame {
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
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
        vec![]
    }

    pub fn get_variables(&mut self, variables_ref: usize) -> Vec<StackFrame> {
        vec![]
    }
}
