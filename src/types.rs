use serde::{Deserialize, Serialize};
use serde_repr::Serialize_repr;
use std::collections::{HashMap, HashSet};
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

#[derive(Debug, Clone, Tsify, Serialize)]
pub struct LocationInfo {
    /// Index into [DebugInfo::files]
    pub file: usize,
    pub line: usize,
    pub col: usize,
    /// Byte offset into the WASM code section for instrumentation
    pub address: usize,
}

/// Debug information parsed from DWARF
#[derive(Debug, Clone, Tsify, Serialize)]
pub struct DebugInfo {
    /// Breakpoint locations (file index, line, col, WASM address).
    pub locations: Vec<LocationInfo>,
    /// Deduplicated source filenames; index matches `LocationInfo::file`.
    pub files: Vec<String>,
    pub functions: Vec<DebugFunction>,

    /// SharedArrayBuffer that controls breakpoint operation in the debugger.
    ///
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
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub memory: js_sys::WebAssembly::Memory,

    /// The debug stack of the executing program
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub stack: js_sys::WebAssembly::Memory,

    /// The raw DWARF binary of this object for use with a debugger
    pub dwarf: Vec<u8>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum WasmOp {
    /// The index of a local in the currently executing function.
    Local(usize),
    /// The index of a global.
    Global(usize),
    /// The index of an item on the operand stack. 0 is the bottom of the operand stack.
    Operand(usize),
}

#[derive(Debug, Clone, Tsify, Serialize)]
pub struct DebugFunction {
    /// Offset of this function in the DWARF
    pub offset: usize,
    /// Code section offset of the start of the function
    pub address: usize,
    /// The total size in bytes of the stack frame, including it's 32-bit tag
    pub size: usize,
    /// The entries in this stack frame
    pub layout: Vec<DebugFrameEntry>,
}

impl DebugFunction {
    /// Clears the layout of the stack frame and resets it to its minimum size.
    pub fn reset(&mut self) {
        self.size = 4; // Space for debug function index tag
        self.layout.clear();
    }

    /// Ensures an entry exists for `loc` and `ty` and returns its offset.
    /// `bkpt` will be added to the lifetime of the found or created entry.
    /// Returns [None] if an entry could not be created (e.g. we cannot store wasm ref types).
    pub fn place(&mut self, loc: WasmOp, ty: wasmparser::ValType, bkpt: usize) -> Option<usize> {
        use wasmparser::ValType;
        if matches!(ty, ValType::Ref(_)) {
            return None;
        }
        if let Some(pos) = self
            .layout
            .iter()
            .position(|e| e.location == loc && e.ty == ty)
        {
            self.layout[pos].lifetime.insert(bkpt);
            return Some(self.layout[pos].offset);
        }

        let offset = self.size;
        let size = match ty {
            ValType::I32 | ValType::F32 => 4,
            ValType::I64 | ValType::F64 => 8,
            ValType::V128 => 16,
            ValType::Ref(_) => unreachable!(),
        };
        self.size += size;
        let mut entry = DebugFrameEntry {
            offset,
            ty,
            location: loc,
            lifetime: HashSet::default(),
        };
        entry.lifetime.insert(bkpt);
        self.layout.push(entry);
        Some(offset)
    }
}

#[derive(Debug, Clone)]
pub struct DebugFrameEntry {
    /// The byte offset of this entry in its containing stack frame
    pub offset: usize,
    /// The WebAssembly type of the value stored by the entry
    pub ty: wasmparser::ValType,
    /// The WebAssembly location (local, global, or stack) represented by this entry's value
    pub location: WasmOp,
    /// A list of breakpoint indices for which this entry is considered valid.
    ///
    /// If a breakpoint index `N` is contained in this list, then accessing this entry's
    /// value immediately after hitting breakpoint `N` will yield a valid value.
    ///
    /// More than one [DebugFrameEntry] in a frame may share the same [DebugFrameEntry::location],
    /// but they are guaranteed to never have overlapping values in [DebugFrameEntry::lifetime].
    /// This permits two frame entries with the same location to contain different types
    /// at different points during the function's execution – for example, `WasmOp::Stack(0)`
    /// might have type [wasmparser::ValType::I32] at the beginning of a function, but change to
    /// [wasmparser::ValType::F64] later on in the function as values are shifted on and off
    /// the operand stack.
    pub lifetime: HashSet<usize>,
}
