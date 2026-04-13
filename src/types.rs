use serde::{Deserialize, Serialize};
use serde_repr::Serialize_repr;
use std::collections::{HashMap, HashSet};
use tsify::Tsify;
use wasm_bindgen::JsValue;
use wasmer::{MemoryType, Pages};
use web_sys::DedicatedWorkerGlobalScope;

use crate::debug::dwarf::{DieReference, Dwarf};

// ============================================================================
// Types
// ============================================================================

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
    /// List of debuggable functions, sorted by low_pc
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
    /// The first address in this function
    pub low_pc: GlobalAddress,
    /// The first address past the end of this function
    pub high_pc: GlobalAddress,
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

// ============================================================================
// Implementations
// ============================================================================

impl DebugInfo {
    /// Finds the index of the function containing this address, if any
    pub fn fn_index_at(&self, pc: GlobalAddress) -> Option<usize> {
        self.functions
            .binary_search_by(|f| {
                if pc < f.low_pc {
                    std::cmp::Ordering::Greater
                } else if pc >= f.high_pc {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .ok()
    }

    /// Finds the function containing this address, if any
    pub fn fn_at(&self, pc: GlobalAddress) -> Option<&DebugFunction> {
        self.fn_index_at(pc).map(|idx| &self.functions[idx])
    }
}

impl DebugFunction {
    pub fn contains(&self, pc: GlobalAddress) -> bool {
        pc >= self.low_pc && pc < self.high_pc
    }

    /// Clears the layout of the stack frame and resets it to its minimum size.
    pub fn reset(&mut self) {
        self.size = 0;
        self.size += 4; // Space for function PC
        self.layout.clear();
    }

    /// Ensures an entry exists for `loc` and returns its offset.
    /// `bkpt` will be added to the lifetime of the found or created entry.
    /// Returns [None] if an entry could not be created (e.g. we cannot store wasm ref types).
    pub fn place(&mut self, location: WasmLocation) -> usize {
        if let Some(pos) = self.layout.iter().position(|e| e.location == location) {
            return self.layout[pos].offset;
        }

        let offset = self.size;
        let size = 8;
        self.size += size;
        self.layout.push(DebugFrameEntry { offset, location });
        offset
    }
}
