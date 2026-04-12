use serde::{Deserialize, Serialize};
use serde_repr::Serialize_repr;
use std::collections::{HashMap, HashSet};
use tsify::Tsify;
use wasm_bindgen::JsValue;
use wasmer::{MemoryType, Pages};
use web_sys::DedicatedWorkerGlobalScope;

use crate::debug::dwarf::{DieReference, Dwarf};

/// Byte offset in the WASM code section
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Tsify, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct GlobalAddress(pub u64);

impl From<u64> for GlobalAddress {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl From<GlobalAddress> for u64 {
    fn from(a: GlobalAddress) -> Self {
        a.0
    }
}

impl From<usize> for GlobalAddress {
    fn from(v: usize) -> Self {
        Self(v as u64)
    }
}

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
    Debug { info: DebugInfo },

    /// Request the main thread to trigger a file download (workers have no document/window).
    #[serde(rename = "download")]
    Download {
        #[tsify(type = "Uint8Array")]
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
        filename: String,
    },

    /// Sent when execution pauses
    #[serde(rename = "breakpoint")]
    Breakpoint,
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

#[derive(Debug, Clone, Tsify, Serialize, Deserialize)]
pub struct LocationInfo {
    /// Index into [DebugInfo::files]
    pub file: usize,
    pub line: usize,
    pub col: usize,
    /// Byte offset into the WASM code section for instrumentation
    pub address: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDescriptor {
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub memory: js_sys::WebAssembly::Memory,
    pub ty: MemoryType,
}

impl MemoryDescriptor {
    pub fn new(initial: u32, maximum: u32) -> Self {
        let desc = js_sys::Object::new();
        js_sys::Reflect::set(&desc, &"initial".into(), &(initial as u32).into()).unwrap();
        js_sys::Reflect::set(&desc, &"maximum".into(), &(maximum as u32).into()).unwrap();
        js_sys::Reflect::set(&desc, &"shared".into(), &true.into()).unwrap();

        let memory = js_sys::WebAssembly::Memory::new(&desc).expect("create WebAssembly.Memory");

        let ty = MemoryType::new(Pages(initial), Some(Pages(maximum)), true);
        Self { memory, ty }
    }
}

/// Debug information parsed from DWARF
#[derive(Debug, Clone, Tsify, Serialize, Deserialize)]
pub struct DebugInfo {
    pub functions: Vec<DebugFunction>,

    /// SharedArrayBuffer that controls breakpoint operation in the debugger.
    ///
    /// - `[u32` Sentinel
    /// - `[u32]` Mode:
    ///   - `0` — Pause on breakpoints
    ///   - `1` — Step into (pause on next location in program order)
    ///   - `2` — Step over (pause when stack depth ≥ current)
    ///   - `3` — Step out (pause when stack depth > current)
    /// - `[u32]` Current breakpoint location
    /// - `[u32]` The saved debug stack pointer (0 if not in breakpoint)
    /// - `[u32] ...` How many times each breakpoint location has been selected by a breakpoint
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub breakpoints: js_sys::SharedArrayBuffer,

    /// The main memory of the executing program
    pub memory: MemoryDescriptor,

    /// The debug stack of the executing program
    pub stack: MemoryDescriptor,

    /// Wrapper around DWARF debug information
    #[serde(with = "crate::debug::dwarf::serde")]
    pub dwarf: Dwarf,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum WasmLocation {
    /// The index of a local in the currently executing function.
    Local(usize),
    /// The index of a global.
    Global(usize),
    /// The index of an item on the operand stack. 0 is the bottom of the operand stack.
    Operand(usize),
}

#[derive(Debug, Clone, Tsify, Serialize, Deserialize)]
pub struct DebugFunction {
    pub address: GlobalAddress,
    /// Reference to dwarf die for this function
    pub die_ref: DieReference,
    /// The total size in bytes of the stack frame, including it's 32-bit tag
    pub size: usize,
    /// The entries in this stack frame
    pub layout: Vec<DebugFrameEntry>,
}

#[derive(Debug, Clone, Tsify, Serialize, Deserialize)]
pub struct DebugFrameEntry {
    /// The byte offset of this entry in its containing stack frame
    pub offset: usize,
    /// The WebAssembly location (local, global, or stack) represented by this entry's value
    pub location: WasmLocation,
}
