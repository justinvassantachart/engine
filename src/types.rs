use serde::{Deserialize, Serialize};
use serde_repr::Serialize_repr;
use std::collections::HashMap;
use tsify::Tsify;
use wasm_bindgen::JsValue;
use wasmparser::MemoryType;
use web_sys::DedicatedWorkerGlobalScope;

#[derive(Debug, Tsify, Deserialize)]
#[serde(untagged)]
pub enum FsNode {
    File(String),
    Dir(HashMap<String, FsNode>),
}

#[derive(Debug, Tsify, Deserialize)]
pub struct WorkerStart {
    pub fs: HashMap<String, FsNode>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub stdin_buffer: js_sys::SharedArrayBuffer,
    pub is_debug: bool,
}

#[derive(Clone, Copy, Debug, Tsify, Serialize_repr)]
#[repr(u8)]
#[tsify(type = "1 | 2")]
pub enum StdoutMode {
    Out = 1,
    Err = 2,
}

#[derive(Tsify, Serialize)]
#[serde(tag = "type")]
pub enum WorkerOut<'a> {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "stdout")]
    Stdout {
        #[tsify(type = "Uint8Array")]
        #[serde(with = "serde_bytes")]
        data: &'a [u8],
        mode: StdoutMode,
    },
    #[serde(rename = "debug")]
    Debug {
        info: DebugInfo,
        /// Bitfield where index N corresponds to breakpoint N.
        /// Index 0 is a sentinel (always 0). Length = breakpoints.len() + 1.
        #[serde(with = "serde_wasm_bindgen::preserve")]
        breakpoint_buffer: js_sys::SharedArrayBuffer,
    },

    /// Request the main thread to trigger a file download (workers have no document/window).
    #[serde(rename = "download")]
    Download {
        #[tsify(type = "Uint8Array")]
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
        filename: String,
    },

    /// Sent when execution pauses at an enabled breakpoint.
    #[serde(rename = "breakpoint")]
    Breakpoint {
        /// 0-based index into the locations array
        location_index: u32,
    },

    #[serde(rename = "stop")]
    Stop,
}

impl<'a> WorkerOut<'a> {
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

#[derive(Debug, Clone, Tsify, Serialize)]
pub struct LocationInfo {
    pub file: u32,
    pub line: u32,
    pub col: u32,
    /// Byte offset into the WASM code section for instrumentation
    pub address: u64,
}

/// Debug information parsed from DWARF
#[derive(Debug, Clone, Default, Tsify, Serialize)]
pub struct DebugInfo {
    /// Breakpoint locations (file index, line, col, WASM address).
    pub locations: Vec<LocationInfo>,
    /// Deduplicated source filenames; index matches `LocationInfo::file`.
    pub files: Vec<String>,
}

impl DebugInfo {
    /// Creates an empty debug info (no locations, no files).
    pub fn new() -> Self {
        Self::default()
    }
}

/// Additional context needed to instantiate and run a binary that has been instrumented.
///
/// This information is recovered while parsing the source binary during the instrumentation pass,
/// rather than the debug info pass, to avoid necessitating an additional `wasmparser` pass during
/// [`crate::dwarf::parse_debug_info`]
pub struct InstrumenterInfo {
    /// Parameters for the memory of the *instrumented* binary
    pub memory: MemoryType,
}
