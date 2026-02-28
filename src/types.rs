use serde::{Deserialize, Serialize};
use serde_repr::Serialize_repr;
use std::collections::HashMap;
use tsify::Tsify;
use wasm_bindgen::JsValue;
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

        /// The main memory of the executing program
        #[serde(with = "serde_wasm_bindgen::preserve")]
        memory: js_sys::WebAssembly::Memory,
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
    /// Index into [DebugInfo::files]
    pub file: u32,
    pub line: u32,
    pub col: u32,
    /// Byte offset into the WASM code section for instrumentation
    pub address: u64,
}

/// Debug information parsed from DWARF
#[derive(Debug, Clone, Default, Tsify, Serialize)]
pub struct DebugInfo {
    pub memory: MemoryInfo,
    /// Breakpoint locations (file index, line, col, WASM address).
    pub locations: Vec<LocationInfo>,
    /// Deduplicated source filenames; index matches `LocationInfo::file`.
    pub files: Vec<String>,
    pub functions: Vec<DebugFunction>,
    pub types: Vec<DebugType>,
}

#[derive(Debug, Clone, Tsify, Serialize)]
pub struct MemoryInfo {
    pub main: wasmer::MemoryType,
    pub debug: wasmer::MemoryType,
}

impl Default for MemoryInfo {
    fn default() -> Self {
        Self {
            main: wasmer::MemoryType::new(1, Some(1), true),
            // Default debug stack size is 64 pages ~ 4MiB
            debug: wasmer::MemoryType::new(64, Some(64), true),
        }
    }
}

/// A single DWARF expression operation, converted from `gimli::Operation`
/// during DWARF parsing. The instrumenter translates these to WASM instructions
/// without depending on gimli.
#[derive(Debug, Clone)]
pub enum DwarfOp {
    /// `DW_OP_fbreg +offset`: push frame_base + offset.
    /// The instrumenter inlines the function's `frame_base` ops, then adds the offset.
    FrameOffset { offset: i64 },

    /// `DW_OP_WASM_location 0x00`: push value of a WASM local.
    WasmLocal { index: u32 },

    /// `DW_OP_stack_value`: marks the expression result as a value, not an address.
    /// Without this, the instrumenter dereferences the result via `i32.load`.
    StackValue,
}

/// A location expression valid over a specific PC range.
/// At `-O0` a variable typically has one range spanning the whole function.
/// At higher optimization levels, DWARF location lists produce multiple ranges
/// as the variable moves between locals, memory, or gets optimized out.
#[derive(Debug, Clone)]
pub struct VarLocationRange {
    /// Code-section-relative start PC (inclusive).
    pub start: u64,
    /// Code-section-relative end PC (exclusive).
    pub end: u64,
    /// DWARF operations that produce the variable's value.
    pub ops: Vec<DwarfOp>,
}

#[derive(Debug, Clone, Tsify, Serialize)]
pub struct DebugFunction {
    pub name: String,
    pub address: usize,
    pub variables: Vec<DebugVariable>,
    /// Size of this function's debug stack frame (bytes).
    pub frame_size: u32,

    /// DWARF expression for the function's frame base (`DW_AT_frame_base`).
    /// Typically `[WasmLocal { index }, StackValue]`.
    #[serde(skip)]
    pub frame_base: Vec<DwarfOp>,
}

#[derive(Debug, Clone, Tsify, Serialize)]
pub struct DebugVariable {
    /// Index into [DebugInfo::types]
    pub ty: u32,
    pub name: String,
    /// Offset of this variable in its containing function's debug stack frame.
    pub frame_offset: u32,

    /// Where and when the variable's value can be read.
    /// Empty if the variable is always optimized out.
    #[serde(skip)]
    pub location: Vec<VarLocationRange>,
}

// TODO(fabio)
#[derive(Debug, Clone, Tsify, Serialize)]
pub struct DebugType {}
