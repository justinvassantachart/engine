use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tsify::Tsify;
use wasm_bindgen::JsValue;
use web_sys::DedicatedWorkerGlobalScope;

#[derive(Debug, Tsify, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FsNode {
    File(String),
    Dir(HashMap<String, FsNode>),
}

#[derive(Debug, Tsify, Serialize, Deserialize)]
pub struct WorkerStart {
    pub fs: HashMap<String, FsNode>,
}

pub enum StdoutMode {
    Out,
    Err,
}

#[derive(Tsify, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WorkerOut {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "stdout")]
    Stdout {
        #[tsify(type = "Uint8Array")]
        data: Vec<u8>,
        mode: StdoutMode,
    },
}

impl WorkerOut {
    pub fn send(&self) {
        let scope = DedicatedWorkerGlobalScope::from(JsValue::from(js_sys::global()));
        scope
            .post_message(
                &serde_wasm_bindgen::to_value(self)
                    .expect("serialization worked")
                    .into(),
            )
            .expect("post_message succeeded");
    }
}
